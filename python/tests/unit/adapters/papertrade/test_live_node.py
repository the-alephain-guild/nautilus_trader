# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------

"""Exercise native factory extraction and the complete LiveNode startup path."""

import asyncio
import json
import threading
import time
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any

import pytest

from nautilus_trader.adapters.papertrade import PAPERTRADE
from nautilus_trader.adapters.papertrade import PapertradeDataClientConfig
from nautilus_trader.adapters.papertrade import PapertradeDataClientFactory
from nautilus_trader.adapters.papertrade import PapertradeExecutionClientConfig
from nautilus_trader.adapters.papertrade import PapertradeExecutionClientFactory
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveExecutionEngineConfig
from nautilus_trader.live import LiveNode
from nautilus_trader.model import AccountId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import TraderId


_FIXTURES = Path(__file__).resolve().parents[5] / "crates/adapters/papertrade/test_data"
_TEST_KEY = "0x" + "11" * 32
_TEST_SIGNER = "0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a"


class _SourceServer(ThreadingHTTPServer):
    wallet: dict[str, Any]
    trading: dict[str, Any]
    closed: threading.Event


class _SourceHandler(BaseHTTPRequestHandler):
    server: _SourceServer

    def log_message(self, _format: str, *_args: object) -> None:
        """Keep signature-bearing requests out of test logs."""

    def reply(self, value: dict[str, Any]) -> None:
        """Write one bounded JSON response."""
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        """Serve metadata, complete history, and a continuous wallet stream."""
        if self.path == "/state/trading":
            self.reply(self.server.trading)
        elif self.path.startswith("/query/accounts/"):
            self.reply({"items": [], "nextCursor": None})
        elif self.path.startswith("/state/user/live?wallet="):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            try:
                self.wfile.write(
                    f"event: wallet\ndata: {json.dumps(self.server.wallet)}\n\n".encode()
                )
                self.wfile.flush()
                while not self.server.closed.wait(0.1):
                    self.wfile.write(b"event: heartbeat\ndata: null\n\n")
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                return
        else:
            self.send_error(404)

    def do_POST(self) -> None:
        """Supply reference quotes or read-only RPC replies; reject every trade."""
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/reference":
            coin = body["coin"]
            bid, ask = ("83000.0", "83000.2") if coin == "BTC" else ("2500.0", "2500.1")
            self.reply(
                {
                    "coin": coin,
                    "time": int(time.time() * 1000),
                    "levels": [[{"px": bid, "sz": "1"}], [{"px": ask, "sz": "1"}]],
                }
            )
        elif self.path == "/rpc":
            self.reply({"jsonrpc": "2.0", "id": 1, "result": []})
        else:
            self.send_error(405, "Trading is disabled in this harness")


@pytest.fixture
def source_server() -> Iterator[tuple[str, dict[str, Any]]]:
    """Serve observed schemas locally, replacing session authorization and clock."""
    server = _SourceServer(("127.0.0.1", 0), _SourceHandler)
    server.wallet = json.loads((_FIXTURES / "live_wallet.json").read_text())
    server.wallet["sessionKeys"] = [[_TEST_SIGNER, str(int(time.time()) + 86400)]]
    server.wallet["indexedThroughTimestamp"] = int(time.time())
    server.trading = json.loads((_FIXTURES / "live_trading.json").read_text())
    server.closed = threading.Event()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/", server.wallet
    finally:
        server.closed.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def build_node(origin: str, wallet: dict[str, Any], path: Path) -> LiveNode:
    """Build the actual native factories, with reconciliation enabled and no strategy."""
    return (
        LiveNode.builder("PAPERTRADE-TEST", TraderId("TESTER-001"), Environment.LIVE)
        .with_timeout_connection(10)
        .with_timeout_reconciliation(10)
        .with_timeout_portfolio(5)
        .with_timeout_disconnection_secs(3)
        .with_delay_post_stop_secs(0)
        .with_delay_shutdown_secs(0)
        .with_exec_engine_config(LiveExecutionEngineConfig(reconciliation=True))
        .add_data_client(
            None,
            PapertradeDataClientFactory(),
            PapertradeDataClientConfig(
                origin=origin,
                rpc_url=origin + "rpc",
                reference_info_url=origin + "reference",
            ),
        )
        .add_exec_client(
            None,
            PapertradeExecutionClientFactory(),
            PapertradeExecutionClientConfig(
                AccountId("PAPERTRADE-001"),
                wallet["address"],
                str(path),
                "test-build",
                session_private_key=_TEST_KEY,
                origin=origin,
                rpc_url=origin + "rpc",
            ),
        )
        .build()
    )


def test_factories_are_registered_with_native_live_node(
    source_server: tuple[str, dict[str, Any]], tmp_path: Path
) -> None:
    """Importing a class is insufficient; building must run the Rust registry extractors."""
    origin, wallet = source_server
    node = build_node(origin, wallet, tmp_path / "journal.redb")
    assert node.environment == Environment.LIVE
    assert PapertradeDataClientFactory().name() == PAPERTRADE
    assert PapertradeExecutionClientFactory().name() == PAPERTRADE


@pytest.mark.asyncio
async def test_live_node_reconciles_both_sides_and_receives_reference_quotes(
    source_server: tuple[str, dict[str, Any]], tmp_path: Path
) -> None:
    """Restore two hedged positions through the actual node, cache, and execution engine."""
    origin, wallet = source_server
    node = build_node(origin, wallet, tmp_path / "journal.redb")
    cache = node.cache
    handle = node.handle()
    task = asyncio.create_task(node.run_async())
    try:

        async def restored() -> tuple[list[Any], Any]:
            while True:
                if task.done():
                    await task
                    raise AssertionError("Node exited before reconciliation")
                positions = cache.positions_open()
                quote = cache.quote(InstrumentId.from_str("BTC-USD-PERP.PAPERTRADE"))
                if len(positions) == 2 and quote is not None:
                    return positions, quote
                await asyncio.sleep(0.02)

        positions, quote = await asyncio.wait_for(restored(), 20)
        assert len({str(position.id) for position in positions}) == 2
        assert {str(position.entry) for position in positions} == {"BUY", "SELL"}
        assert quote.bid_size.as_decimal() == 0
        assert quote.ask_size.as_decimal() == 0
        assert cache.account(AccountId("PAPERTRADE-001")) is not None
    finally:
        handle.stop()
        await asyncio.wait_for(task, 15)
