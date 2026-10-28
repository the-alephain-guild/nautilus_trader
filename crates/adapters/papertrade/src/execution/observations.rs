// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

//! Serial account refresh, stream reconciliation, and bounded observation work.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Result, ensure};
use nautilus_core::UUID4;
use tokio_util::sync::CancellationToken;

use super::Inner;
use crate::{api::WalletStream, reconciliation::synchronize_with_epoch, wire::AccountSnapshot};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(20);

struct AcceptedAccount {
    snapshot: AccountSnapshot,
    epoch: UUID4,
}

impl AcceptedAccount {
    async fn apply(
        &mut self,
        inner: &Inner,
        snapshot: AccountSnapshot,
        epoch: UUID4,
    ) -> Result<bool> {
        snapshot.validate(inner.wallet)?;
        let successes: Vec<_> = snapshot
            .intent_outcomes
            .iter()
            .filter(|(_, outcome)| outcome == "succeeded")
            .map(|(id, _)| *id)
            .collect();
        if !successes.is_empty() {
            inner.journal.observe_successes(&successes)?;
        }
        let previous = &self.snapshot;
        let regresses = snapshot.indexed_through_block < previous.indexed_through_block
            || snapshot.indexed_through_timestamp < previous.indexed_through_timestamp
            || snapshot.wallet_revision < previous.wallet_revision
            || snapshot.queue_revision < previous.queue_revision;
        let advances = snapshot.indexed_through_block > previous.indexed_through_block
            || snapshot.indexed_through_timestamp > previous.indexed_through_timestamp
            || snapshot.wallet_revision > previous.wallet_revision
            || snapshot.queue_revision > previous.queue_revision;
        if epoch != self.epoch && regresses && !advances {
            log::debug!("papertrade_account_frame_superseded source_epoch={epoch}");
            return Ok(false);
        }
        ensure!(
            !regresses,
            "Account source coverage conflicts with the accepted observation"
        );
        synchronize_with_epoch(inner, snapshot.clone(), epoch).await?;
        self.snapshot = snapshot;
        self.epoch = epoch;
        Ok(true)
    }
}

async fn bounded<T>(
    cancel: &CancellationToken,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => anyhow::bail!("Account observation cancelled"),
        result = tokio::time::timeout(OBSERVATION_TIMEOUT, operation) => {
            result.map_err(|_| anyhow::anyhow!("Account observation exceeded its total deadline"))?
        }
    }
}

pub(super) async fn run(
    inner: Arc<Inner>,
    mut stream: WalletStream,
    snapshot: AccountSnapshot,
    mut stream_epoch: UUID4,
    cancel: CancellationToken,
) {
    let mut accepted = AcceptedAccount {
        snapshot,
        epoch: stream_epoch,
    };
    let mut poll = tokio::time::interval(Duration::from_millis(inner.config.poll_interval_ms));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let refresh = tokio::time::sleep(REFRESH_INTERVAL);
    tokio::pin!(refresh);
    'connected: loop {
        // Keep the same stream read and idle deadline while other work makes progress.
        let result = {
            let incoming = stream.next();
            tokio::pin!(incoming);
            loop {
                let fresh = tokio::select! {
                    () = cancel.cancelled() => break 'connected,
                    result = &mut incoming => break result,
                    () = &mut refresh => {
                        inner.account_refresh_requested.swap(false, Ordering::AcqRel);
                        true
                    }
                    _ = poll.tick() => inner.account_refresh_requested.swap(false, Ordering::AcqRel),
                };
                let result = Box::pin(bounded(&cancel, async {
                    if fresh {
                        let epoch = UUID4::new();
                        let snapshot = inner.api.snapshot(inner.wallet).await?;
                        accepted.apply(&inner, snapshot, epoch).await
                    } else {
                        let snapshot = accepted.snapshot.clone();
                        accepted.apply(&inner, snapshot, accepted.epoch).await
                    }
                }))
                .await;
                if fresh {
                    refresh
                        .as_mut()
                        .reset(tokio::time::Instant::now() + REFRESH_INTERVAL);
                }
                match result {
                    Ok(true) if fresh => inner.account_source_fault.store(false, Ordering::Release),
                    Err(e) => {
                        inner.account_source_fault.store(true, Ordering::Release);
                        log::error!("papertrade_reconciliation_unavailable error={e}");
                    }
                    _ => {}
                }
            }
        };
        match result {
            Ok(snapshot) => {
                match Box::pin(bounded(
                    &cancel,
                    accepted.apply(&inner, snapshot, stream_epoch),
                ))
                .await
                {
                    Ok(true) => inner.account_source_fault.store(false, Ordering::Release),
                    Err(e) => {
                        inner.account_source_fault.store(true, Ordering::Release);
                        log::error!("papertrade_reconciliation_unavailable error={e}");
                    }
                    Ok(false) => {}
                }
            }
            Err(e) => {
                inner.account_source_fault.store(true, Ordering::Release);
                inner.stream_healthy.store(false, Ordering::Release);
                inner.live.store(false, Ordering::Release);
                log::warn!("papertrade_wallet_reconnecting error={e}");
                loop {
                    tokio::select! {
                        () = cancel.cancelled() => break 'connected,
                        () = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                    let replacement = tokio::select! {
                        biased;
                        () = cancel.cancelled() => break 'connected,
                        result = tokio::time::timeout(OBSERVATION_TIMEOUT, async {
                            let epoch = UUID4::new();
                            let mut stream = inner.api.wallet_stream(inner.wallet).await?;
                            let snapshot = stream.next().await?;
                            let applied = accepted.apply(&inner, snapshot, epoch).await?;
                            Ok::<_, anyhow::Error>((stream, epoch, applied))
                        }) => result.map_err(|_| anyhow::anyhow!("Account reconnect exceeded its total deadline")).and_then(|result| result),
                    };
                    match replacement {
                        Ok((replacement, epoch, applied)) => {
                            *inner.stream_activity.write() = Some(replacement.activity.clone());
                            stream = replacement;
                            stream_epoch = epoch;
                            inner.stream_healthy.store(true, Ordering::Release);
                            if applied {
                                inner.account_source_fault.store(false, Ordering::Release);
                            } else {
                                inner.live.store(false, Ordering::Release);
                            }
                            refresh
                                .as_mut()
                                .reset(tokio::time::Instant::now() + REFRESH_INTERVAL);
                            break;
                        }
                        Err(e) => log::warn!("papertrade_wallet_connect_failed error={e}"),
                    }
                }
            }
        }
    }
    inner.stream_healthy.store(false, Ordering::Release);
    inner.live.store(false, Ordering::Release);
}
