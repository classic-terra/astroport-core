use crate::error::ContractError;
use crate::state::{Config, BRIDGES};
use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::maker::ExecuteMsg;
use astroport::pair::Cw20HookMsg;
use astroport::querier::query_pair_info;
use cosmwasm_std::{to_json_binary, Coin, Deps, Env, StdResult, SubMsg, Uint128, WasmMsg};

/// The default bridge depth for a fee token
pub const BRIDGES_INITIAL_DEPTH: u64 = 0;
/// Maximum amount of bridges to use in a multi-hop swap
pub const BRIDGES_MAX_DEPTH: u64 = 2;
/// Swap execution depth limit
pub const BRIDGES_EXECUTION_MAX_DEPTH: u64 = 3;

pub fn try_build_swap_msg(
    deps: Deps,
    cfg: &Config,
    from: AssetInfo,
    to: AssetInfo,
    amount_in: Uint128,
) -> Result<SubMsg, ContractError> {
    let pool = get_pool(deps, cfg, from.clone(), to)?;
    let msg = build_swap_msg(deps, cfg, pool, from, amount_in)?;
    Ok(msg)
}

pub fn build_swap_msg(
    deps: Deps,
    cfg: &Config,
    pool: PairInfo,
    from: AssetInfo,
    amount_in: Uint128,
) -> Result<SubMsg, ContractError> {
    if from.is_native_token() {
        let mut offer_asset = Asset {
            info: from.clone(),
            amount: amount_in,
        };

        // Deduct tax first
        let amount_in = amount_in.checked_sub(offer_asset.compute_tax(&deps.querier)?)?;

        offer_asset.amount = amount_in;

        Ok(SubMsg::new(WasmMsg::Execute {
            contract_addr: pool.contract_addr.to_string(),
            msg: to_json_binary(&astroport::pair::ExecuteMsg::Swap {
                offer_asset,
                belief_price: None,
                max_spread: Some(cfg.max_spread),
                to: None,
            })?,
            funds: vec![Coin {
                denom: from.to_string(),
                amount: amount_in,
            }],
        }))
    } else {
        Ok(SubMsg::new(WasmMsg::Execute {
            contract_addr: from.to_string(),
            msg: to_json_binary(&cw20::Cw20ExecuteMsg::Send {
                contract: pool.contract_addr.to_string(),
                amount: amount_in,
                msg: to_json_binary(&Cw20HookMsg::Swap {
                    belief_price: None,
                    max_spread: Some(cfg.max_spread),
                    to: None,
                })?,
            })?,
            funds: vec![],
        }))
    }
}

pub fn build_distribute_msg(
    env: Env,
    bridge_assets: Vec<AssetInfo>,
    depth: u64,
) -> StdResult<SubMsg> {
    let msg = if !bridge_assets.is_empty() {
        // Swap bridge assets
        SubMsg::new(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::SwapBridgeAssets {
                assets: bridge_assets,
                depth,
            })?,
            funds: vec![],
        })
    } else {
        // Update balances and distribute rewards
        SubMsg::new(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::DistributeAstro {})?,
            funds: vec![],
        })
    };

    Ok(msg)
}

pub fn validate_bridge(
    deps: Deps,
    cfg: &Config,
    from_token: AssetInfo,
    bridge_token: AssetInfo,
    astro_token: AssetInfo,
    depth: u64,
) -> Result<PairInfo, ContractError> {
    // Check if the bridge pool exists
    let bridge_pool = get_pool(deps, cfg, from_token.clone(), bridge_token.clone())?;

    // Check if the bridge token - ASTRO pool exists
    let astro_pool = get_pool(deps, cfg, bridge_token.clone(), astro_token.clone());
    if astro_pool.is_err() {
        if depth >= BRIDGES_MAX_DEPTH {
            return Err(ContractError::MaxBridgeDepth(depth));
        }

        // Check if next level of bridge exists
        let next_bridge_token = BRIDGES
            .load(deps.storage, bridge_token.to_string())
            .map_err(|_| ContractError::InvalidBridgeDestination(from_token.clone()))?;

        validate_bridge(
            deps,
            cfg,
            bridge_token,
            next_bridge_token,
            astro_token,
            depth + 1,
        )?;
    }

    Ok(bridge_pool)
}

pub fn get_pool(
    deps: Deps,
    cfg: &Config,
    from: AssetInfo,
    to: AssetInfo,
) -> Result<PairInfo, ContractError> {
    query_pair_info(
        &deps.querier,
        cfg.factory_contract.clone(),
        &[from.clone(), to.clone()],
    )
    .map_err(|_| ContractError::InvalidBridgeNoPool(from.clone(), to.clone()))
}
