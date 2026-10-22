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

"""Exercise local signed intents through LiveNode, its execution engine and cold recovery."""

import asyncio
import copy
import json
import sys
import threading
import time
from collections.abc import Callable
from collections.abc import Iterator
from decimal import Decimal
from pathlib import Path
from typing import Any
from typing import override

import pytest

from nautilus_trader.model import AccountId
from nautilus_trader.model import ClientOrderId
from nautilus_trader.model import Currency
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import OrderSide
from nautilus_trader.model import OrderStatus
from nautilus_trader.model import PositionId
from nautilus_trader.model import Quantity
from nautilus_trader.model import StrategyId
from nautilus_trader.model import TimeInForce
from nautilus_trader.trading import Strategy
from nautilus_trader.trading import StrategyConfig
from tests.unit.adapters.papertrade.test_live_node import _FIXTURES
from tests.unit.adapters.papertrade.test_live_node import _TEST_SIGNER
from tests.unit.adapters.papertrade.test_live_node import _SourceHandler
from tests.unit.adapters.papertrade.test_live_node import _SourceServer
from tests.unit.adapters.papertrade.test_live_node import build_node


_BTC = InstrumentId.from_str("BTC-USD-PERP.PAPERTRADE")
_OPEN = ClientOrderId("PT-LIVE-OPEN")
_CLOSE = ClientOrderId("PT-LIVE-CLOSE")
_POSITION = PositionId("PT-9001")
_TX = "0x" + "31" * 32
_BLOCK_HASH = "0x" + "32" * 32
_RESULT_TOPIC = "0x8a23ef1120f3e42e722a0c0a599a66d55ab2650ecc39716f57e625684e349cad"
_CLOSE_TOPIC = "0x87902ed2451849f465fc4d36a01d7f5c67b53af12f63ee85d2814a2d582f89bf"


class _TradingProbe(Strategy):
    def __init__(self) -> None:
        super().__init__(
            StrategyConfig(
                strategy_id=StrategyId("PapertradeProbe-001"),
                order_id_tag="001",
                external_order_instrument_ids=[_BTC],
            )
        )
        self.allow_close = False

    def on_start(self) -> None:
        self.subscribe_quotes(_BTC)

    def on_quote(self, _quote: Any) -> None:
        factory = self.order_factory
        if self.cache.order(_OPEN) is None:
            self.submit_order(
                factory.market(
                    _BTC,
                    OrderSide.BUY,
                    Quantity.from_str("1660"),
                    time_in_force=TimeInForce.GTC,
                    quote_quantity=True,
                    client_order_id=_OPEN,
                )
            )
        elif self.allow_close and self.cache.order(_CLOSE) is None:
            self.submit_order(
                factory.market(
                    _BTC,
                    OrderSide.SELL,
                    Quantity.from_str("0.02"),
                    time_in_force=TimeInForce.GTC,
                    reduce_only=True,
                    client_order_id=_CLOSE,
                ),
                position_id=_POSITION,
            )


class _Scenario:
    def __init__(self) -> None:
        self.lock = threading.RLock()
        self.wallet = json.loads((_FIXTURES / "live_wallet.json").read_text())
        self.wallet.update(
            openPositions=[],
            locked="0",
            balance=str(1000 * 10**18),
            indexedThroughBlock=49135000,
            indexedThroughTimestamp=int(time.time()),
            walletRevision=49135000,
            sessionKeys=[[_TEST_SIGNER, str(int(time.time()) + 86400)]],
        )
        self.wallet["funds"] = {
            "reservedBalance": "0",
            "reservedQueued": "0",
            "availableBalance": str(1000 * 10**18),
            "availableQueued": "0",
        }
        self.trading = json.loads((_FIXTURES / "live_trading.json").read_text())
        self.trading["indexedThroughBlock"] = self.wallet["indexedThroughBlock"]
        self.posts: list[dict[str, Any]] = []
        self.ids: dict[str, str] = {}
        self.history: list[dict[str, Any]] = []
        self.receipt_ready = False
        self.opened_ms = 0

    def advance(self) -> None:
        self.wallet["accountRevision"] += 1
        self.wallet["indexedThroughBlock"] += 1
        self.wallet["walletRevision"] = self.wallet["indexedThroughBlock"]
        self.wallet["indexedThroughTimestamp"] = int(time.time())
        self.trading["indexedThroughBlock"] = self.wallet["indexedThroughBlock"]

    def open(self) -> None:
        with self.lock:
            self.advance()
            self.opened_ms = self.wallet["indexedThroughTimestamp"] * 1000
            self.wallet["openPositions"] = [
                [
                    "9001",
                    0,
                    True,
                    False,
                    "830000",
                    str(166 * 10**18),
                    10,
                    "0",
                    self.opened_ms,
                    self.ids["openPosition"],
                ]
            ]
            self.wallet["balance"] = str(834 * 10**18)
            self.wallet["locked"] = str(166 * 10**18)
            self.wallet["funds"]["availableBalance"] = self.wallet["balance"]

    def close(self) -> None:
        with self.lock:
            self.advance()
            self.wallet["openPositions"] = []
            self.wallet["locked"] = "0"
            self.wallet["balance"] = str(101825 * 10**16)
            self.wallet["funds"]["availableBalance"] = self.wallet["balance"]
            self.history = [
                {
                    "positionId": "9001",
                    "instrumentId": 0,
                    "symbol": "BTC",
                    "priceScaleRaw": "10",
                    "isLong": True,
                    "entryPriceRaw": "830000",
                    "marginRaw": str(166 * 10**18),
                    "leverage": 10,
                    "openedAtMs": self.opened_ms,
                    "timestampMs": self.wallet["indexedThroughTimestamp"] * 1000,
                    "txHash": _TX,
                    "kind": "PositionClosed",
                    "exitPriceRaw": "840000",
                    "adjustedPnlRaw": str(19 * 10**18),
                    "userPaidFeeRaw": str(5 * 10**17),
                    "builderFeeRaw": str(25 * 10**16),
                }
            ]

    def receipt(self) -> dict[str, Any]:
        block = hex(self.wallet["indexedThroughBlock"])
        base = {
            "transactionHash": _TX,
            "blockHash": _BLOCK_HASH,
            "blockNumber": block,
            "removed": False,
        }
        result = dict(
            base,
            address="0x233ff29b239abc1d7748605da9b205c3e008626e",
            topics=[_RESULT_TOPIC, "0x" + "00" * 32, "0x" + "00" * 32, self.ids["closePositions"]],
            data="0x" + f"{1:064x}",
        )
        close = dict(
            base,
            address="0x6cd5661646289fb6e65ea5c032310fded797d0a2",
            topics=[
                _CLOSE_TOPIC,
                "0x" + f"{9001:064x}",
                "0x" + f"{int(self.wallet['address'], 16):064x}",
            ],
            data="0x" + "".join(f"{n:064x}" for n in (840000, 0, 19 * 10**18, 0, 5 * 10**17)),
        )
        return {
            "status": "0x1",
            "transactionHash": _TX,
            "blockHash": _BLOCK_HASH,
            "blockNumber": block,
            "logs": [result, close],
        }


class _ExecutionServer(_SourceServer):
    scenario: _Scenario


class _ExecutionHandler(_SourceHandler):
    server: _ExecutionServer

    @override
    def do_GET(self) -> None:
        scenario = self.server.scenario
        if self.path.startswith("/query/accounts/"):
            with scenario.lock:
                self.reply({"items": copy.deepcopy(scenario.history), "nextCursor": None})
        elif self.path == "/state/trading":
            with scenario.lock:
                self.reply(copy.deepcopy(scenario.trading))
        elif self.path.startswith("/state/user/live?"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            previous = None
            try:
                while not self.server.closed.is_set():
                    with scenario.lock:
                        payload = json.dumps(scenario.wallet)
                    if payload != previous:
                        self.wfile.write(f"event: wallet\ndata: {payload}\n\n".encode())
                        previous = payload
                    else:
                        self.wfile.write(b"event: heartbeat\ndata: null\n\n")
                    self.wfile.flush()
                    self.server.closed.wait(0.05)
            except (BrokenPipeError, ConnectionResetError):
                return
        else:
            super().do_GET()

    @override
    def do_POST(self) -> None:
        if self.path == "/reference":
            super().do_POST()
            return
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        scenario = self.server.scenario
        with scenario.lock:
            if self.path == "/intents":
                scenario.posts.append(body)
                # The local venue observes the signed request but loses its acknowledgement.
                self.send_error(500, "Acknowledgement lost in local harness")
            elif self.path == "/rpc":
                method = body["method"]
                result: Any = None
                if method == "eth_getLogs":
                    query = body["params"][0]
                    if scenario.posts:
                        action = scenario.posts[-1]["action"]
                        for intent in query["topics"][3]:
                            scenario.ids.setdefault(action, intent)
                    result = []
                    if scenario.receipt_ready:
                        receipt = scenario.receipt()
                        height = int(receipt["blockNumber"], 16)
                        if int(query["fromBlock"], 16) <= height <= int(query["toBlock"], 16):
                            result = [
                                log
                                for log in receipt["logs"]
                                if log["topics"][0] == _RESULT_TOPIC
                                and log["topics"][3] in query["topics"][3]
                            ]
                elif method == "eth_getTransactionReceipt" and scenario.receipt_ready:
                    result = scenario.receipt()
                self.reply({"jsonrpc": "2.0", "id": body["id"], "result": result})
            else:
                self.send_error(405, "Unsupported local trade route")


@pytest.fixture
def execution_source() -> Iterator[tuple[str, _Scenario]]:
    """Serve all venue operations on loopback with a deterministic payoff."""
    server = _ExecutionServer(("127.0.0.1", 0), _ExecutionHandler)
    server.scenario = _Scenario()
    server.wallet = server.scenario.wallet
    server.trading = server.scenario.trading
    server.closed = threading.Event()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/", server.scenario
    finally:
        server.closed.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


async def _until(predicate: Callable[[], Any], task: asyncio.Task[Any]) -> Any:
    async def ready() -> Any:
        while True:
            if task.done():
                await task
                raise AssertionError("LiveNode exited before the expected cache state")
            value = predicate()
            if value:
                return value
            await asyncio.sleep(0.02)

    return await asyncio.wait_for(ready(), 20)


@pytest.mark.asyncio
async def test_live_node_keeps_local_close_identity_pnl_and_cold_restart_cache(
    execution_source: tuple[str, _Scenario],
    tmp_path: Path,
) -> None:
    """Apply real engine fills once and recover the same economics into a fresh cache."""
    origin, source = execution_source
    path = tmp_path / "journal.redb"
    node = build_node(origin, source.wallet, path)
    probe = _TradingProbe()
    node.add_strategy(probe)
    cache = node.cache
    handle = node.handle()
    task = asyncio.create_task(node.run_async())
    try:
        await _until(lambda: source.ids.get("openPosition"), task)
        assert len(source.posts) == 1
        assert source.posts[0]["signed"]["size"] == str(166 * 10**18)
        source.open()
        await _until(
            lambda: (
                cache.order(_OPEN) is not None and cache.order(_OPEN).status == OrderStatus.FILLED
            ),
            task,
        )
        position = cache.position(_POSITION)
        assert position.quantity.as_decimal() == Decimal("0.02")
        assert not cache.order(_OPEN).is_quote_quantity
        probe.allow_close = True
        await _until(lambda: source.ids.get("closePositions"), task)
        source.close()
        # Force at least one reconciliation cycle to observe history without its receipt.
        await asyncio.sleep(3.5)
        assert cache.order(_CLOSE).status != OrderStatus.FILLED
        assert all(
            not str(order.client_order_id).startswith("PT-CLOSE-9001") for order in cache.orders()
        )
        source.receipt_ready = True
        await _until(lambda: cache.order(_CLOSE).status == OrderStatus.FILLED, task)
        position = cache.position(_POSITION)
        assert position.is_closed
        assert not cache.positions_open()
        assert position.realized_pnl.as_decimal() == Decimal("18.25")
        assert sum((fee.as_decimal() for fee in position.commissions()), Decimal(0)) == Decimal(
            "1.75"
        )
        assert len(position.trade_ids()) == 2
        assert len(cache.orders()) == 2
        assert str(cache.order(_CLOSE).venue_order_id) == source.ids["closePositions"]
        assert len(source.posts) == 2
        account = cache.account(AccountId("PAPERTRADE-001"))
        assert account.balance_total(Currency.from_str("USDC")).as_decimal() == Decimal("1018.25")
        assert set(position.trade_ids()) == {
            trade for order in cache.orders() for trade in order.trade_ids
        }
        assert all(
            order.strategy_id == StrategyId("PapertradeProbe-001") for order in cache.orders()
        )
        expected_trade_ids = set(position.trade_ids())
        expected_order_trades = {
            str(order.client_order_id): tuple(order.trade_ids) for order in cache.orders()
        }
    finally:
        handle.stop()
        await asyncio.wait_for(task, 15)
        node.dispose()
    del node, probe, cache, handle, task

    restarted = build_node(origin, source.wallet, path)
    restarted.add_strategy(_TradingProbe())
    cache = restarted.cache
    handle = restarted.handle()
    task = asyncio.create_task(restarted.run_async())
    try:
        await _until(
            lambda: (
                cache.order(_CLOSE) is not None and cache.order(_CLOSE).status == OrderStatus.FILLED
            ),
            task,
        )
        position = cache.position(_POSITION)
        assert position.is_closed
        assert not cache.positions_open()
        assert position.realized_pnl.as_decimal() == Decimal("18.25")
        assert len(position.trade_ids()) == 2
        assert len(cache.orders()) == 2
        assert str(cache.order(_CLOSE).venue_order_id) == source.ids["closePositions"]
        assert len(source.posts) == 2
        account = cache.account(AccountId("PAPERTRADE-001"))
        assert account.balance_total(Currency.from_str("USDC")).as_decimal() == Decimal("1018.25")
        assert set(position.trade_ids()) == {
            trade for order in cache.orders() for trade in order.trade_ids
        }
        assert sum((fee.as_decimal() for fee in position.commissions()), Decimal(0)) == Decimal(
            "1.75"
        )
        assert all(
            order.quantity.as_decimal() == Decimal("0.02")
            and order.filled_qty.as_decimal() == Decimal("0.02")
            and not order.is_quote_quantity
            for order in cache.orders()
        )
        assert set(position.trade_ids()) == expected_trade_ids
        assert {
            str(order.client_order_id): tuple(order.trade_ids) for order in cache.orders()
        } == expected_order_trades
        assert all(
            order.strategy_id == StrategyId("PapertradeProbe-001") for order in cache.orders()
        )
    finally:
        handle.stop()
        await asyncio.wait_for(task, 15)


@pytest.mark.asyncio
async def test_live_node_recovers_an_unknown_order_after_process_kill_without_resubmitting(
    execution_source: tuple[str, _Scenario],
    tmp_path: Path,
) -> None:
    """Kill a real node after a lost acknowledgement and restore its fill in a fresh process state."""
    origin, source = execution_source
    path = tmp_path / "killed.redb"
    program = (
        "import json,sys; "
        "from tests.unit.adapters.papertrade.test_execution_recovery import _TradingProbe,build_node; "
        "node=build_node(sys.argv[1],json.loads(sys.argv[3]),sys.argv[2]); "
        "node.add_strategy(_TradingProbe()); node.run()"
    )
    child = await asyncio.create_subprocess_exec(
        sys.executable,
        "-c",
        program,
        origin,
        str(path),
        json.dumps(source.wallet),
        cwd=str(_FIXTURES.parents[3] / "python"),
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.PIPE,
    )
    exited = asyncio.create_task(child.wait())
    diagnostics = b""
    try:
        await _until(lambda: source.ids.get("openPosition"), exited)
        assert len(source.posts) == 1
    finally:
        if child.returncode is None:
            child.kill()
        await asyncio.wait_for(exited, 5)
        if child.stderr is not None:
            diagnostics = await child.stderr.read()
    assert child.returncode != 0, diagnostics.decode(errors="replace")
    source.open()
    restarted = build_node(origin, source.wallet, path)
    restarted.add_strategy(_TradingProbe())
    cache = restarted.cache
    handle = restarted.handle()
    task = asyncio.create_task(restarted.run_async())
    try:
        await _until(
            lambda: (
                cache.order(_OPEN) is not None and cache.order(_OPEN).status == OrderStatus.FILLED
            ),
            task,
        )
        position = cache.position(_POSITION)
        assert position.quantity.as_decimal() == Decimal("0.02")
        assert len(position.trade_ids()) == 1
        assert len(cache.orders()) == 1
        assert str(cache.order(_OPEN).venue_order_id) == source.ids["openPosition"]
        await asyncio.sleep(3.5)
        assert len(source.posts) == 1
        assert len(cache.positions_open()) == 1
    finally:
        handle.stop()
        await asyncio.wait_for(task, 15)
