from __future__ import annotations

import asyncio
import typing
from dataclasses import dataclass

from src.logs import Logger
from src.logs import LogLevel

logger = Logger()


@dataclass
class DexMetrics:
    name: str = "dex"

    # latency and gas
    latency: float = 0
    latency_ema: float = 0

    block_timestamp: int = 0
    gas_spent: float = 0
    gas_spent_last: float = 0
    gas_spent_ema: float = 0

    pool_fee = 0.0005  # POUTATOUDOU
    fee_last: float = 0
    fee_ema: float = 0

    # price metrics
    tx_counter: int = 0
    tx_counter_last: int = 0

    tx_price: float | None = None
    tx_price_last: float | None = None

    pool_price_decimal: float | None = None
    pool_price_decimal_last: float | None = None

    pool_price: float | None = None
    pool_price_last: float | None = None
    sqrt_price: float | None = None

    tick_current: float | None = None  # should be an int
    tick_last: float | None = None
    tick_ema: float | None = None

    log_return: float = 0
    log_return_last: float = 0
    log_return_ema: float = 0

    # volume metrics in base
    volume: float = 0
    volume_last: float = 0
    volume_ema: float = 1.0

    volume_synth_ema: float = 0.0

    trade_flow: float = 0
    trade_flow_last: float = 0
    trade_flow_ema: float = 0

    # variance metrics
    variation_pool: float = 0
    variation_pool_last: float = 0
    variation_pool_ema: float = 1e-4  # to avoid dividing by 0.

    variance: float = 0
    variance_last: float = 0
    variance_ema: float = 1e-8  # to avoid dividing by 0.
    variance_tx: float = 0
    variance_tx_last: float = 0
    variance_tx_ema: float = 1e-8  # to avoid dividing by 0.
    variance_pool: float = 0
    variance_pool_last: float = 0
    variance_pool_ema: float = 1e-8  # to avoid dividing by 0.
    variance_tick: float = 0
    variance_tick_last: float = 0
    variance_tick_ema: float = 1e-8  # to avoid dividing by 0.

    # liquidity metrics
    liquidity_real: float = 0
    liquidity_real_ema: float = 0

    # from transaction
    liquidity_real_tx: float = 0
    liquidity_real_tx_ema: float = 0

    # from state
    liquidity_real_pool: float = 0
    liquidity_real_pool_ema: float = 0
    liquidity_real_tick: float = 0
    liquidity_real_tick_ema: float = 0

    # liquidity implied
    liquidity_uniswap: int = 0
    liquidity_implied_swap: float = 0
    liquidity_implied_swap_last: float | None = None
    liquidity_implied_swap_ema: float = 0

    liquidity_implied_pool: float = 0
    liquidity_implied_pool_last: float | None = None
    liquidity_implied_pool_ema: float = 0
    # liquidity metrics 2
    power: float = 0
    power_last: float = 0
    power_ema: float = 0

    # performance metrics
    pool_value: float = 0
    pool_leverage: float = 0
    performance: float = 0
    performance_ema: float = 0
    pnl_theo: float = 0.0
    pnl_theo_ema: float = 0
    pnl_return: float = 0
    pnl_return_ema: float | None = None
    vega: float = 0
    vega_ema: float | None = None

    def feature_sample(self, freq_time: float, ema: float) -> dict[str, typing.Any]:
        # delta
        log_return_delta = self.log_return - self.log_return_last
        # tick_delta = self.tick_current - self.tick_last
        tx_delta = self.tx_counter - self.tx_counter_last
        volume_delta = self.volume - self.volume_last
        volume_synth_delta = self.liquidity_implied_pool * abs(log_return_delta)
        trade_flow_delta = self.trade_flow - self.trade_flow_last

        variation_pool_delta = self.variation_pool - self.variation_pool_last
        variance_delta = self.variance - self.variance_last
        variance_tx_delta = self.variance_tx - self.variance_tx_last
        variance_pool_delta = self.variance_pool - self.variance_pool_last
        variance_tick_delta = self.variance_tick - self.variance_tick_last

        if self.liquidity_implied_pool_last:
            self.pnl_theo = self.pool_fee * volume_delta - 0.5 * self.liquidity_implied_pool_last * variance_pool_delta
            if volume_delta > 0:
                self.performance = self.pnl_theo / volume_delta
            else:
                self.performance = 0
            self.pnl_return = self.pnl_theo / self.pool_value

        if variance_delta > 0:
            self.liquidity_real = volume_delta / variance_delta
        if variance_tx_delta > 0:
            self.liquidity_real_tx = volume_delta / variance_tx_delta
        if variance_pool_delta > 0:
            self.liquidity_real_pool = volume_delta / variance_pool_delta
            self.vega = self.pnl_theo / variance_pool_delta
        if variance_tick_delta > 0:
            self.liquidity_real_tick = volume_delta / variance_tick_delta

        # liquidity_imp_delta = self.liquidity_imp - self.liquidity_imp_last
        if not self.tick_ema:
            self.tick_ema = self.tick_current
        if not self.pnl_return_ema:
            self.pnl_return_ema = self.pnl_return
        # ema
        self.log_return_ema = log_return_delta * ema + self.log_return_ema * (1 - ema)
        if self.tick_current and self.tick_ema:
            self.tick_ema = self.tick_current * ema + self.tick_ema * (1 - ema)
        self.volume_ema = volume_delta * ema + self.volume_ema * (1 - ema)
        self.volume_synth_ema = volume_synth_delta * ema + self.volume_synth_ema * (1 - ema)
        self.trade_flow_ema = trade_flow_delta * ema + self.trade_flow_ema * (1 - ema)

        self.variation_pool_ema = variation_pool_delta * ema + self.variation_pool * (1 - ema)
        self.variance_ema = variance_delta * ema + self.variance_ema * (1 - ema)
        self.variance_tx_ema = variance_tx_delta * ema + self.variance_tx_ema * (1 - ema)
        self.variance_pool_ema = variance_pool_delta * ema + self.variance_pool_ema * (1 - ema)
        self.variance_tick_ema = variance_tick_delta * ema + self.variance_tick_ema * (1 - ema)

        self.liquidity_implied_swap_ema = self.liquidity_implied_swap * ema + self.liquidity_implied_swap_ema * (1 - ema)
        self.liquidity_implied_pool_ema = self.liquidity_implied_pool * ema + self.liquidity_implied_pool_ema * (1 - ema)

        self.liquidity_real_ema = self.volume_ema / self.variance_ema
        self.liquidity_real_tx_ema = self.volume_ema / self.variance_tx_ema
        self.liquidity_real_pool_ema = self.volume_ema / self.variance_pool_ema
        self.liquidity_real_tick_ema = self.volume_ema / self.variance_tick_ema

        self.pnl_theo_ema = self.pool_fee * self.volume_ema - 0.5 * self.liquidity_implied_pool_ema * self.variance_pool_ema
        self.performance_ema = self.pnl_theo_ema / self.volume_ema
        self.pnl_return_ema = self.pnl_theo_ema / self.pool_value
        self.vega_ema = self.pnl_theo_ema / self.variance_pool_ema

        # reset
        self.log_return_last = self.log_return
        self.tx_counter_last = self.tx_counter
        self.volume_last = self.volume
        self.trade_flow_last = self.trade_flow

        self.variation_pool_last = self.variation_pool
        self.variance_last = self.variance
        self.variance_tx_last = self.variance_tx
        self.variance_pool_last = self.variance_pool
        self.variance_tick_last = self.variance_tick

        self.liquidity_implied_pool_last = self.liquidity_implied_pool
        self.liquidity_implied_swap_last = self.liquidity_implied_swap

        return {
            "metrics": self.name,
            "tx_counter": tx_delta,
            "log_return_delta": round(log_return_delta, 5),
            "volume_delta": round(volume_delta, 8),
            "volume_ema": round(self.volume_ema, 2),
            "volume_synth_ema": round(self.volume_synth_ema, 2),
            # "trade_flow_ema": round(self.trade_flow_ema, 2),
            "variation_pool_delta": round(variation_pool_delta, 5),
            "variation_pool_ema": round(self.variation_pool_ema, 5),
            "variance_delta": round(variance_delta, 10),
            "variance_tx_delta": round(variance_tx_delta, 10),
            "variance_pool_delta": round(variance_pool_delta, 10),
            "variance_tick_delta": round(variance_tick_delta, 10),
            # "variance_ema": round(self.variance_ema, 10),
            "variance_tx_ema": round(self.variance_tx_ema, 10),
            "variance_pool_ema": round(self.variance_pool_ema, 10),
            "variance_tick_ema": round(self.variance_tick_ema, 10),
            "liquidity_real_ema": round(self.liquidity_real_ema, 2),
            "liquidity_real_tx_ema": round(self.liquidity_real_tx_ema, 2),
            "liquidity_real_pool_ema": round(self.liquidity_real_pool_ema, 2),
            "liquidity_real_tick_ema": round(self.liquidity_real_tick_ema, 2),
            "liquidity_imp_swap": round(self.liquidity_implied_swap, 2),
            "liquidity_imp_pool": round(self.liquidity_implied_pool, 2),
            "performance": round(self.performance, 10),
            "performance_ema": round(self.performance_ema, 10),
            "pnl_theo": round(self.pnl_theo, 6),
            "pnl_theo_ema": round(self.pnl_theo_ema, 6),
            "pnl_return": round(self.pnl_return, 10),
            "pnl_return_ema": round(self.pnl_return_ema, 10),
            "vega": round(self.vega, 2),
            "vega_ema": round(self.vega_ema, 2),
            "pool_price": round(self.pool_price, 8) if self.pool_price else None,
            "pool_price_decimal": self.pool_price_decimal,
            "pool_value_in_token_1": self.pool_value,
            "pool_value_in_token_0": self.pool_value / self.pool_price if self.pool_price else None,
            "pool_leverage": self.pool_leverage,
            "tick_current": self.tick_current,
        }

    async def feature_stream(self, callbacks_async: list[typing.Callable], freq_time: float, ema: float = 0.01):  # type: ignore
        while True:
            await asyncio.sleep(freq_time)
            for function_n in callbacks_async:  # type: ignore
                await function_n()
            metrics = self.feature_sample(freq_time=freq_time, ema=ema)
            logger.log(level=LogLevel.info, msg={"msg": "dex_metrics", **metrics})


@dataclass
class CexMetrics:
    name: str = "cex"
    # latency
    latency: float = 0
    latency_ema: float = 0
    # metrics

    log_return: float = 0
    log_return_last: float = 0
    log_return_ema: float = 0

    spread: float = 0

    variance: float = 0
    variance_last: float = 0
    variance_ema: float = 1e-10  # to avoid dividing by 0.

    variance2_ema: float = 1e-10  # to avoid dividing by 0.

    variance_felt: float = 0
    variance_felt_last: float = 0
    variance_felt_ema: float = 1e-10  # to avoid dividing by 0.

    slippage: float = 0
    slippage_last: float = 0
    slippage_ema: float = 1e-5

    slippage_sq: float = 0
    slippage_sq_last: float = 0
    slippage_sq_ema: float = 1e-10

    variation: float = 0
    variation_last: float = 0
    variation_ema: float = 1e-5

    skew: float = 0
    skew_last: float = 0
    skew_ema: float = 0

    volume: float = 0
    volume_last: float = 0
    volume_ema: float = 0

    trade_flow: float = 0
    trade_flow_ema: float = 0

    volume_retail: float = 0
    volume_retail_up: float = 0
    volume_retail_down: float = 0
    volume_retail_last: float = 0
    volume_retail_up_last: float = 0
    volume_retail_down_last: float = 0
    volume_retail_ema: float = 0
    volume_retail_up_ema: float = 0
    volume_retail_down_ema: float = 0

    trade_flow_retail: float = 0
    trade_flow_retail_ema: float = 0

    micro_cluster_log: float = 0
    micro_cluster_size: float = 0
    micro_cluster_volume: float = 0
    micro_cluster_geom: float = 0
    micro_cluster_geom_sq: float = 0

    antiselek: float = 0
    antiselek_last: float = 0
    antiselek_ema: float = 0

    power_ema: float = 0
    power2_ema: float = 0

    # q_learning
    micro_cluster_count: int = 0
    micro_cluster_count_last: int = 0
    micro_cluster_count_ema: float = 0

    # this assumes we are buying the full inventory
    micro_price_hot: float = 0
    micro_price_hot_last: float = 0
    micro_price_hot_ema: float = 0

    micro_price_hot_variance: float = 0
    micro_price_hot_variance_last: float = 0
    micro_price_hot_variance_ema: float = 0

    pnl_hot: float = 0
    pnl_hot_last: float = 0
    pnl_hot_ema: float = 0

    mean_reversion_hot: float = 0
    mean_reversion_hot_last: float = 0
    mean_reversion_hot_ema: float = 0

    # this assumes we are buying a fixed amount of the inventory (ema)
    micro_price_warm: float = 0
    micro_price_warm_last: float = 0
    micro_price_warm_ema: float = 0

    micro_price_warm_variance: float = 0
    micro_price_warm_variance_last: float = 0
    micro_price_warm_variance_ema: float = 0

    pnl_warm: float = 0
    pnl_warm_last: float = 0
    pnl_warm_ema: float = 0

    mean_reversion_warm: float = 0
    mean_reversion_warm_last: float = 0
    mean_reversion_warm_ema: float = 0

    # this assumes we are counting on reversion (ema -> 1-ema)
    micro_price_cold: float = 0
    micro_price_cold_last: float = 0
    micro_price_cold_ema: float = 0

    micro_price_cold_variance: float = 0
    micro_price_cold_variance_last: float = 0
    micro_price_cold_variance_ema: float = 0

    pnl_cold: float = 0
    pnl_cold_last: float = 0
    pnl_cold_ema: float = 0

    mean_reversion: float = 0
    mean_reversion_last: float = 0
    mean_reversion_ema: float = 0

    mean_reversion_cold: float = 0
    mean_reversion_cold_last: float = 0
    mean_reversion_cold_ema: float = 0

    # RL metrics for the gradient
    q_pnl: float = 0
    q_pnl_last: float = 0
    q_pnl_ema: float = 0
    q_pnl2_ema: float = 0

    q_perf_ema: float = 0

    # micro cluster metrics if mom >> 0 it means micro trades form clusters
    momentum_ema: float = 0

    # new
    size_up: float = 0
    size_dw: float = 0
    flow_up: float = 0
    flow_dw: float = 0
    var_up: float = 0
    var_dw: float = 0

    # new
    size_up_last: float = 0
    size_dw_last: float = 0
    flow_up_last: float = 0
    flow_dw_last: float = 0
    var_up_last: float = 0
    var_dw_last: float = 0

    size_up_ema: float = 0
    size_dw_ema: float = 0
    flow_up_ema: float = 0
    flow_dw_ema: float = 0
    var_up_ema: float = 1e-8
    var_dw_ema: float = 1e-8

    # liquidity features
    micro_liquidity: float = 0  # testing the micro liquidity concept

    liquidity_real: float = 0
    liquidity_real_ema: float = 0
    liq_up_ema: float = 0
    liq_dw_ema: float = 0

    liquidity_imp: float = 0
    liquidity_imp_ema: float = 0
    liquidity_imbalance: float = 0
    liquidity_imbalance_ema: float = 0

    # OG metrics
    smile: float = 0
    smile_ema: float = 0

    def market_feature_sample(self, ema: float = 0.01):
        # increment 11
        micro_cluster_count_delta = self.micro_cluster_count - self.micro_cluster_count_last
        log_return_delta = self.log_return - self.log_return_last
        variation_delta = self.variation - self.variation_last
        variance_delta = self.variance - self.variance_last
        variance_felt_delta = self.variance_felt - self.variance_felt_last

        skew_delta = self.skew - self.skew_last

        volume_delta = self.volume - self.volume_last
        volume_retail_up_delta = self.volume_retail_up - self.volume_retail_up_last
        volume_retail_down_delta = self.volume_retail_down - self.volume_retail_down_last
        volume_retail_delta = self.volume_retail - self.volume_retail_last
        antiselek_delta = self.antiselek - self.antiselek_last
        slippage_delta = self.slippage - self.slippage_last
        slippage_sq_delta = self.slippage_sq - self.slippage_sq_last

        # pnl and momentum 11
        # q_pnl_delta = self.q_pnl - self.q_pnl_last

        micro_price_hot_variance_delta = self.micro_price_hot_variance - self.micro_price_hot_variance_last
        micro_price_warm_variance_delta = self.micro_price_warm_variance - self.micro_price_warm_variance_last
        micro_price_cold_variance_delta = self.micro_price_cold_variance - self.micro_price_cold_variance_last
        mean_reversion_delta = self.mean_reversion - self.mean_reversion_last
        mean_reversion_hot_delta = self.mean_reversion_hot - self.mean_reversion_hot_last
        mean_reversion_warm_delta = self.mean_reversion_warm - self.mean_reversion_warm_last
        mean_reversion_cold_delta = self.mean_reversion_cold - self.mean_reversion_cold_last
        pnl_hot_delta = self.pnl_hot - self.pnl_hot_last
        pnl_warm_delta = self.pnl_warm - self.pnl_warm_last
        pnl_cold_delta = self.pnl_cold - self.pnl_cold_last

        # liquidity_real = 0
        # if variance_delta > 0:
        #     liquidity_real = volume_delta / variance_delta

        # new delta 6
        flow_up_delta = self.flow_up - self.flow_up_last
        size_up_delta = self.size_up - self.size_up_last
        var_up_delta = self.var_up - self.var_up_last
        flow_dw_delta = self.flow_dw - self.flow_dw_last
        size_dw_delta = self.size_dw - self.size_dw_last
        var_dw_delta = self.var_dw - self.var_dw_last

        # ema
        self.micro_cluster_count_ema += ema * (micro_cluster_count_delta - self.micro_cluster_count_ema)
        self.log_return_ema += ema * (log_return_delta - self.log_return_ema)

        self.variance_ema += ema * (variance_delta - self.variance_ema)
        self.variance_felt_ema += ema * (variance_felt_delta - self.variance_felt_ema)
        self.variance2_ema += ema * (log_return_delta**2 - self.variance2_ema)
        if self.variance_ema > 0:
            self.momentum_ema = self.variance2_ema / self.variance_ema
        self.variation_ema += ema * (variation_delta - self.variation_ema)

        self.skew_ema += ema * (skew_delta - self.skew_ema)

        self.volume_ema += ema * (volume_delta - self.volume_ema)
        self.volume_retail_ema += ema * (volume_retail_delta - self.volume_retail_ema)
        self.volume_retail_up_ema += ema * (volume_retail_up_delta - self.volume_retail_up_ema)
        self.volume_retail_down_ema += ema * (volume_retail_down_delta - self.volume_retail_down_ema)

        # a revoir pourquoi pas de delta ici?
        self.trade_flow_ema += ema * (self.trade_flow - self.trade_flow_ema)
        self.trade_flow_retail_ema += ema * (self.trade_flow_retail - self.trade_flow_retail_ema)

        self.slippage_ema += ema * (slippage_delta - self.slippage_ema)
        self.slippage_sq_ema += ema * (slippage_sq_delta - self.slippage_sq_ema)
        self.antiselek_ema += ema * (antiselek_delta - self.antiselek_ema)

        # pnl and momentum
        self.q_pnl_ema = 0.00005 * self.volume_ema - self.liquidity_real_ema * self.skew_ema / (self.power_ema + 1)
        self.q_pnl2_ema = self.volume_ema * (0.00005 - self.variation_ema / (self.power_ema + 1))
        if self.volume_ema > 0:
            self.q_perf_ema = self.q_pnl_ema / self.volume_ema

        self.micro_price_hot_variance_ema += ema * (micro_price_hot_variance_delta - self.micro_price_hot_variance_ema)
        self.micro_price_warm_variance_ema += ema * (micro_price_warm_variance_delta - self.micro_price_warm_variance_ema)
        self.micro_price_cold_variance_ema += ema * (micro_price_cold_variance_delta - self.micro_price_cold_variance_ema)
        self.mean_reversion_ema += ema * (mean_reversion_delta - self.mean_reversion_ema)
        self.mean_reversion_hot_ema += ema * (mean_reversion_hot_delta - self.mean_reversion_hot_ema)
        self.mean_reversion_warm_ema += ema * (mean_reversion_warm_delta - self.mean_reversion_warm_ema)
        self.mean_reversion_cold_ema += ema * (mean_reversion_cold_delta - self.mean_reversion_cold_ema)
        self.pnl_hot_ema += ema * (pnl_hot_delta - self.pnl_hot_ema)
        self.pnl_warm_ema += ema * (pnl_warm_delta - self.pnl_warm_ema)
        self.pnl_cold_ema += ema * (pnl_cold_delta - self.pnl_cold_ema)

        self.flow_up_ema += ema * (flow_up_delta - self.flow_up_ema)
        self.flow_dw_ema += ema * (flow_dw_delta - self.flow_dw_ema)
        self.size_up_ema += ema * (size_up_delta - self.size_up_ema)
        self.size_dw_ema += ema * (size_dw_delta - self.size_dw_ema)
        self.var_up_ema += ema * (var_up_delta - self.var_up_ema)
        self.var_dw_ema += ema * (var_dw_delta - self.var_dw_ema)

        self.liquidity_real_ema = self.volume_ema / self.variance_ema
        self.liq_up_ema = self.flow_up_ema / self.var_up_ema
        self.liq_dw_ema = self.flow_dw_ema / self.var_dw_ema

        slip = self.slippage_ema / self.variation_ema
        if slip < 1:
            self.power_ema = slip / (1 - slip)
        slip_sq = self.slippage_sq_ema / self.variance_ema
        if slip_sq < 1:
            self.power2_ema = 2 * slip_sq / (1 - slip_sq)

        # no delta here?
        self.liquidity_imp_ema = ema * (self.liquidity_imp - self.liquidity_imp_ema)
        self.liquidity_imbalance_ema = ema * (self.liquidity_imbalance - self.liquidity_imbalance_ema)

        self.smile_ema = self.var_up_ema - self.var_dw_ema

        # reset
        self.micro_cluster_count_last = self.micro_cluster_count
        self.log_return_last = self.log_return
        self.variation_last = self.variation
        self.variance_last = self.variance
        self.variance_felt_last = self.variance_felt

        self.skew_last = self.skew

        self.volume_last = self.volume
        self.volume_retail_last = self.volume_retail
        self.volume_retail_up_last = self.volume_retail_up
        self.volume_retail_down_last = self.volume_retail_down

        self.antiselek_last = self.antiselek
        self.slippage_last = self.slippage
        self.slippage_sq_last = self.slippage_sq

        self.q_pnl_last = self.q_pnl

        self.micro_price_hot_variance_last = self.micro_price_hot_variance
        self.micro_price_warm_variance_last = self.micro_price_warm_variance
        self.micro_price_cold_variance_last = self.micro_price_cold_variance
        self.mean_reversion_last = self.mean_reversion
        self.mean_reversion_hot_last = self.mean_reversion_hot
        self.mean_reversion_warm_last = self.mean_reversion_warm
        self.mean_reversion_cold_last = self.mean_reversion_cold
        self.pnl_hot_last = self.pnl_hot
        self.pnl_warm_last = self.pnl_warm
        self.pnl_cold_last = self.pnl_cold

        # new
        self.size_up_last = self.size_up
        self.size_dw_last = self.size_dw
        self.flow_up_last = self.flow_up
        self.flow_dw_last = self.flow_dw
        self.var_up_last = self.var_up
        self.var_dw_last = self.var_dw

        msg: dict[str, typing.Any] = {
            "metrics": self.name,
            "count_delta": micro_cluster_count_delta,
            "log_return_delta": round(log_return_delta, 5),
            "volume_delta": round(volume_delta, 0),
            "volume_retail_delta": round(volume_retail_delta),
            "volume_ema": round(self.volume_ema, 2),
            "variance_delta": round(variance_delta, 10),
            "variance_ema": round(self.variance_ema, 10),
            "momemtum_ema": round(self.momentum_ema, 4),
            "variance_felt_ema": round(self.variance_felt_ema, 10),
            # "variance2_ema": round(self.variance2_ema, 10),
            # "variation_delta": round(variation_delta, 5),
            # "variation_ema": round(self.variation_ema, 5),
            ##"skew_ema": round(self.skew_ema, 15),
            "liquidity_real_ema": round(0.0001 * self.liquidity_real_ema, 2),
            "liq_up_ema": round(0.0001 * self.liq_up_ema, 2),
            "liq_dw_ema": round(0.0001 * self.liq_dw_ema, 2),
            # "liq_imp_ema": round(self.liquidity_imp_ema, 2),
            "slippage": round(slip, 4),
            "power": round(self.power_ema, 2),
            # "slippage_sq": round(slip_sq, 4),
            # "power2": round(self.power2_ema, 2),
            # "q_pnl_ema": round(0.01 * self.q_pnl_ema, 2),
            # "q_perf_ema": round(self.q_perf_ema, 5),
            # "q_pnl2_ema": round(0.01 * self.q_pnl2_ema, 2),
            # "q_perf2_ema": round(self.q_perf_ema / (0.01 + self.volume_ema), 5),
            # "hot_pnl": round(self.pnl_hot_ema, 5),
            # "warm_pnl": round(self.pnl_warm_ema, 5),
            # "cold_pnl": round(self.pnl_cold_ema, 5),
            # "antiselek_ema": round(self.antiselek_ema, 2),
            # "antiselek_yield": round(self.antiselek_ema / self.volume_ema, 6),
        }

        logger.log(level=LogLevel.info, msg=msg)

    async def market_feature_stream(self, freq_time: float, ema: float = 0.01):
        while True:
            await asyncio.sleep(freq_time)
            self.market_feature_sample(ema=ema)
