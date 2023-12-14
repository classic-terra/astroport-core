use crate::error::ContractError;
use crate::state::{Config, CONFIG};

use cosmwasm_std::{
    attr, entry_point, from_json, to_json_binary, Addr, Binary, Coin, CosmosMsg, Decimal, Deps,
    DepsMut, Env, MessageInfo, Reply, ReplyOn, Response, StdError, StdResult, SubMsg, Uint128,
    WasmMsg, Decimal256, Uint256
};

use crate::response::MsgInstantiateContractResponse;
use astroport::asset::{addr_validate_to_lower, format_lp_token_name, Asset, AssetInfo, PairInfo};
use astroport::factory::PairType;
use astroport::generator::Cw20HookMsg as GeneratorHookMsg;
use astroport::pair::{ConfigResponse, DEFAULT_SLIPPAGE, MAX_ALLOWED_SLIPPAGE};
use astroport::pair::{
    CumulativePricesResponse, Cw20HookMsg, ExecuteMsg, InstantiateMsg, MigrateMsg, PoolResponse,
    QueryMsg, ReverseSimulationResponse, SimulationResponse, TWAP_PRECISION,
};
use astroport::querier::{query_factory_config, query_fee_info, query_supply};
use astroport::{token::InstantiateMsg as TokenInstantiateMsg, U256};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use protobuf::Message;
use std::convert::TryFrom;
use std::ops::Mul;
use std::str::FromStr;
use std::vec;

/// Contract name that is used for migration.
const CONTRACT_NAME: &str = "astroport-pair";
/// Contract version that is used for migration.
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// A `reply` call code ID of sub-message.
const INSTANTIATE_TOKEN_REPLY_ID: u64 = 1;

/// ## Description
/// Creates a new contract with the specified parameters in the [`InstantiateMsg`].
/// Returns the [`Response`] with the specified attributes if the operation was successful, or a [`ContractError`] if the contract was not created
/// ## Params
/// * **deps** is the object of type [`DepsMut`].
///
/// * **env** is the object of type [`Env`].
///
/// * **_info** is the object of type [`MessageInfo`].
/// * **msg** is a message of type [`InstantiateMsg`] which contains the basic settings for creating a contract
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    msg.asset_infos[0].check(deps.api)?;
    msg.asset_infos[1].check(deps.api)?;

    if msg.asset_infos[0] == msg.asset_infos[1] {
        return Err(ContractError::DoublingAssets {});
    }

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let config = Config {
        pair_info: PairInfo {
            contract_addr: env.contract.address.clone(),
            liquidity_token: Addr::unchecked(""),
            asset_infos: msg.asset_infos.clone(),
            pair_type: PairType::Xyk {},
        },
        factory_addr: addr_validate_to_lower(deps.api, msg.factory_addr.as_str())?,
        block_time_last: 0,
        price0_cumulative_last: Uint128::zero(),
        price1_cumulative_last: Uint128::zero(),
    };

    CONFIG.save(deps.storage, &config)?;

    let token_name = format_lp_token_name(msg.asset_infos, &deps.querier)?;

    // Create LP token
    let sub_msg: Vec<SubMsg> = vec![SubMsg {
        msg: WasmMsg::Instantiate {
            code_id: msg.token_code_id,
            msg: to_json_binary(&TokenInstantiateMsg {
                name: token_name,
                symbol: "uLP".to_string(),
                decimals: 6,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: env.contract.address.to_string(),
                    cap: None,
                }),
                marketing: None,
            })?,
            funds: vec![],
            admin: None,
            label: String::from("Astroport LP token"),
        }
        .into(),
        id: INSTANTIATE_TOKEN_REPLY_ID,
        gas_limit: None,
        reply_on: ReplyOn::Success,
    }];

    Ok(Response::new().add_submessages(sub_msg))
}

/// # Description
/// The entry point to the contract for processing the reply from the submessage
/// # Params
/// * **deps** is the object of type [`DepsMut`].
///
/// * **_env** is the object of type [`Env`].
///
/// * **msg** is the object of type [`Reply`].
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    if config.pair_info.liquidity_token != Addr::unchecked("") {
        return Err(ContractError::Unauthorized {});
    }

    let data = msg.result.unwrap().data.unwrap();
    let res: MsgInstantiateContractResponse =
        Message::parse_from_bytes(data.as_slice()).map_err(|_| {
            StdError::parse_err("MsgInstantiateContractResponse", "failed to parse data")
        })?;

    config.pair_info.liquidity_token =
        addr_validate_to_lower(deps.api, res.get_contract_address())?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attribute("liquidity_token_addr", config.pair_info.liquidity_token))
}

/// ## Description
/// Available the execute messages of the contract.
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **env** is the object of type [`Env`].
///
/// * **info** is the object of type [`MessageInfo`].
///
/// * **msg** is the object of type [`ExecuteMsg`].
///
/// ## Queries
/// * **ExecuteMsg::UpdateConfig { params: Binary }** Not supported.
///
/// * **ExecuteMsg::Receive(msg)** Receives a message of type [`Cw20ReceiveMsg`] and processes
/// it depending on the received template.
///
/// * **ExecuteMsg::ProvideLiquidity {
///             assets,
///             slippage_tolerance,
///             auto_stake,
///             receiver,
///         }** Provides liquidity with the specified input parameters.
///
/// * **ExecuteMsg::Swap {
///             offer_asset,
///             belief_price,
///             max_spread,
///             to,
///         }** Performs an swap operation with the specified parameters.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateConfig { .. } => Err(ContractError::NonSupported {}),
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
        ExecuteMsg::ProvideLiquidity {
            assets,
            slippage_tolerance,
            auto_stake,
            receiver,
        } => provide_liquidity(
            deps,
            env,
            info,
            assets,
            slippage_tolerance,
            auto_stake,
            receiver,
        ),
        ExecuteMsg::Swap {
            offer_asset,
            belief_price,
            max_spread,
            to,
        } => {
            offer_asset.info.check(deps.api)?;
            if !offer_asset.is_native_token() {
                return Err(ContractError::Unauthorized {});
            }

            let to_addr = if let Some(to_addr) = to {
                Some(addr_validate_to_lower(deps.api, &to_addr)?)
            } else {
                None
            };

            swap(
                deps,
                env,
                info.clone(),
                info.sender,
                offer_asset,
                belief_price,
                max_spread,
                to_addr,
            )
        }
    }
}

/// ## Description
/// Receives a message of type [`Cw20ReceiveMsg`] and processes it depending on the received template.
/// If the template is not found in the received message, then an [`ContractError`] is returned,
/// otherwise returns the [`Response`] with the specified attributes if the operation was successful
/// ## Params
/// * **deps** is the object of type [`DepsMut`].
///
/// * **env** is the object of type [`Env`].
///
/// * **info** is the object of type [`MessageInfo`].
///
/// * **cw20_msg** is the object of type [`Cw20ReceiveMsg`].
pub fn receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    let contract_addr = info.sender.clone();
    match from_json(&cw20_msg.msg) {
        Ok(Cw20HookMsg::Swap {
            belief_price,
            max_spread,
            to,
        }) => {
            // only asset contract can execute this message
            let mut authorized: bool = false;
            let config: Config = CONFIG.load(deps.storage)?;

            for pool in config.pair_info.asset_infos {
                if let AssetInfo::Token { contract_addr, .. } = &pool {
                    if contract_addr == &info.sender {
                        authorized = true;
                    }
                }
            }

            if !authorized {
                return Err(ContractError::Unauthorized {});
            }

            let to_addr = if let Some(to_addr) = to {
                Some(addr_validate_to_lower(deps.api, to_addr.as_str())?)
            } else {
                None
            };

            swap(
                deps,
                env,
                info,
                Addr::unchecked(cw20_msg.sender),
                Asset {
                    info: AssetInfo::Token { contract_addr },
                    amount: cw20_msg.amount,
                },
                belief_price,
                max_spread,
                to_addr,
            )
        }
        Ok(Cw20HookMsg::WithdrawLiquidity {}) => withdraw_liquidity(
            deps,
            env,
            info,
            Addr::unchecked(cw20_msg.sender),
            cw20_msg.amount,
        ),
        Err(err) => Err(ContractError::Std(err)),
    }
}

/// ## Description
/// Provides liquidity with the specified input parameters.
/// Returns an [`ContractError`] on failure, otherwise returns the [`Response`] with the specified
/// attributes if the operation was successful.
/// ## Params
/// * **deps** is the object of type [`DepsMut`].
///
/// * **env** is the object of type [`Env`].
///
/// * **info** is the object of type [`MessageInfo`].
///
/// * **slippage_tolerance** is an [`Option`] field of type [`Decimal`]. Used for sets the maximum
/// percent of price movement.
///
/// * **auto_stake** is an [`Option`] field of type [`bool`]. Determines whether an autostake will
/// be performed on the generator.
///
/// * **receiver** is an [`Option`] field of type  [`String`]. Sets the receiver of liquidity.
// CONTRACT - should approve contract to use the amount of token.
pub fn provide_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    assets: [Asset; 2],
    slippage_tolerance: Option<Decimal>,
    auto_stake: Option<bool>,
    receiver: Option<String>,
) -> Result<Response, ContractError> {
    assets[0].info.check(deps.api)?;
    assets[1].info.check(deps.api)?;

    let auto_stake = auto_stake.unwrap_or(false);
    for asset in assets.iter() {
        asset.assert_sent_native_token_balance(&info)?;
    }

    let mut config: Config = CONFIG.load(deps.storage)?;
    let mut pools: [Asset; 2] = config
        .pair_info
        .query_pools(&deps.querier, env.contract.address.clone())?;
    let deposits: [Uint128; 2] = [
        assets
            .iter()
            .find(|a| a.info.equal(&pools[0].info))
            .map(|a| a.amount)
            .expect("Wrong asset info is given"),
        assets
            .iter()
            .find(|a| a.info.equal(&pools[1].info))
            .map(|a| a.amount)
            .expect("Wrong asset info is given"),
    ];

    if deposits[0].is_zero() || deposits[1].is_zero() {
        return Err(ContractError::InvalidZeroAmount {});
    }

    let mut messages: Vec<CosmosMsg> = vec![];
    for (i, pool) in pools.iter_mut().enumerate() {
        // If the pool is token contract, then we need to execute TransferFrom msg to receive funds
        if let AssetInfo::Token { contract_addr, .. } = &pool.info {
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: contract_addr.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::TransferFrom {
                    owner: info.sender.to_string(),
                    recipient: env.contract.address.to_string(),
                    amount: deposits[i],
                })?,
                funds: vec![],
            }));
        } else {
            // If the asset is native token, balance is already increased
            // To calculated properly we should subtract user deposit from the pool
            pool.amount = pool.amount.checked_sub(deposits[i])?;
        }
    }

    let total_share = query_supply(&deps.querier, config.pair_info.liquidity_token.clone())?;
    let share = if total_share.is_zero() {
        // Initial share = collateral amount
        Uint128::new(
            (U256::from(deposits[0].u128()) * U256::from(deposits[1].u128()))
                .integer_sqrt()
                .as_u128(),
        )
    } else {
        // assert slippage tolerance
        assert_slippage_tolerance(slippage_tolerance, &deposits, &pools)?;

        // min(1, 2)
        // 1. sqrt(deposit_0 * exchange_rate_0_to_1 * deposit_0) * (total_share / sqrt(pool_0 * pool_1))
        // == deposit_0 * total_share / pool_0
        // 2. sqrt(deposit_1 * exchange_rate_1_to_0 * deposit_1) * (total_share / sqrt(pool_1 * pool_1))
        // == deposit_1 * total_share / pool_1
        std::cmp::min(
            deposits[0].multiply_ratio(total_share, pools[0].amount),
            deposits[1].multiply_ratio(total_share, pools[1].amount),
        )
    };

    // mint LP token for sender or receiver if set
    let receiver = receiver.unwrap_or_else(|| info.sender.to_string());
    messages.extend(mint_liquidity_token_message(
        deps.as_ref(),
        &config,
        env.clone(),
        addr_validate_to_lower(deps.api, receiver.as_str())?,
        share,
        auto_stake,
    )?);

    // Accumulate prices for oracle
    if let Some((price0_cumulative_new, price1_cumulative_new, block_time)) =
        accumulate_prices(env, &config, pools[0].amount, pools[1].amount)?
    {
        config.price0_cumulative_last = price0_cumulative_new;
        config.price1_cumulative_last = price1_cumulative_new;
        config.block_time_last = block_time;
        CONFIG.save(deps.storage, &config)?;
    }

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "provide_liquidity"),
        attr("sender", info.sender.as_str()),
        attr("receiver", receiver.as_str()),
        attr("assets", format!("{}, {}", assets[0], assets[1])),
        attr("share", share.to_string()),
    ]))
}

/// # Description
/// Mint LP token to beneficiary or auto deposit into generator if set.
/// # Params
/// * **deps** is the object of type [`Deps`].
///
/// * **config** is the object of type [`Config`].
///
/// * **env** is the object of type [`Env`].
///
/// * **recipient** is the object of type [`Addr`]. The recipient of the liquidity.
///
/// * **amount** is the object of type [`Uint128`]. The amount that will be mint to the recipient.
///
/// * **auto_stake** is the field of type [`bool`]. Determines whether an autostake will be performed on the generator
fn mint_liquidity_token_message(
    deps: Deps,
    config: &Config,
    env: Env,
    recipient: Addr,
    amount: Uint128,
    auto_stake: bool,
) -> Result<Vec<CosmosMsg>, ContractError> {
    let lp_token = config.pair_info.liquidity_token.clone();

    // If no auto-stake - just mint to recipient
    if !auto_stake {
        return Ok(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lp_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Mint {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        })]);
    }

    // Mint to contract and stake to generator
    let generator =
        query_factory_config(&deps.querier, config.clone().factory_addr)?.generator_address;

    if generator.is_none() {
        return Err(ContractError::AutoStakeError {});
    }

    Ok(vec![
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lp_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Mint {
                recipient: env.contract.address.to_string(),
                amount,
            })?,
            funds: vec![],
        }),
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lp_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Send {
                contract: generator.unwrap().to_string(),
                amount,
                msg: to_json_binary(&GeneratorHookMsg::DepositFor(recipient))?,
            })?,
            funds: vec![],
        }),
    ])
}

/// ## Description
/// Withdrawing liquidity from the pool. Returns an [`ContractError`] on failure,
/// otherwise returns the [`Response`] with the specified attributes if the operation was successful.
/// ## Params
/// * **deps** is the object of type [`DepsMut`].
///
/// * **env** is the object of type [`Env`].
///
/// * **info** is the object of type [`MessageInfo`].
///
/// * **sender** is the object of type [`Addr`]. Sets where liquidity will be withdrawn.
///
/// * **amount** is the object of type [`Uint128`]. Sets the withdrawal amount.
pub fn withdraw_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sender: Addr,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage).unwrap();

    if info.sender != config.pair_info.liquidity_token {
        return Err(ContractError::Unauthorized {});
    }

    let (pools, total_share) = pool_info(deps.as_ref(), config.clone())?;
    let refund_assets = get_share_in_assets(&pools, amount, total_share);

    // Accumulate prices for oracle
    if let Some((price0_cumulative_new, price1_cumulative_new, block_time)) =
        accumulate_prices(env, &config, pools[0].amount, pools[1].amount)?
    {
        config.price0_cumulative_last = price0_cumulative_new;
        config.price1_cumulative_last = price1_cumulative_new;
        config.block_time_last = block_time;
        CONFIG.save(deps.storage, &config)?;
    }

    // update pool info
    let messages: Vec<CosmosMsg> = vec![
        refund_assets[0]
            .clone()
            .into_msg(&deps.querier, sender.clone())?,
        refund_assets[1]
            .clone()
            .into_msg(&deps.querier, sender.clone())?,
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.pair_info.liquidity_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Burn { amount })?,
            funds: vec![],
        }),
    ];

    let attributes = vec![
        attr("action", "withdraw_liquidity"),
        attr("sender", sender.as_str()),
        attr("withdrawn_share", &amount.to_string()),
        attr(
            "refund_assets",
            format!("{}, {}", refund_assets[0], refund_assets[1]),
        ),
    ];

    Ok(Response::new()
        .add_messages(messages)
        .add_attributes(attributes))
}

/// ## Description
/// Returns the share of assets.
/// ## Params
/// * **pools** are an array of [`Asset`] type items.
///
/// * **amount** is the object of type [`Uint128`].
///
/// * **total_share** is the object of type [`Uint128`].
pub fn get_share_in_assets(
    pools: &[Asset; 2],
    amount: Uint128,
    total_share: Uint128,
) -> Vec<Asset> {
    let mut share_ratio = Decimal::zero();
    if !total_share.is_zero() {
        share_ratio = Decimal::from_ratio(amount, total_share);
    }

    pools
        .iter()
        .map(|a| Asset {
            info: a.info.clone(),
            amount: a.amount * share_ratio,
        })
        .collect()
}

/// ## Description
/// Performs an swap operation with the specified parameters. CONTRACT - a user must do token approval.
/// Returns an [`ContractError`] on failure, otherwise returns the [`Response`] with the specified attributes if the operation was successful.
/// ## Params
/// * **deps** is the object of type [`DepsMut`].
///
/// * **env** is the object of type [`Env`].
///
/// * **info** is the object of type [`MessageInfo`].
///
/// * **sender** is the object of type [`Addr`]. Sets the default recipient of the swap operation.
///
/// * **offer_asset** is the object of type [`Asset`]. Proposed asset for swapping.
///
/// * **belief_price** is the object of type [`Option<Decimal>`]. Used to calculate the maximum spread.
///
/// * **max_spread** is the object of type [`Option<Decimal>`]. Sets the maximum spread of the swap operation.
///
/// * **to** is the object of type [`Option<Addr>`]. Sets the recipient of the swap operation.
#[allow(clippy::too_many_arguments)]
pub fn swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sender: Addr,
    offer_asset: Asset,
    belief_price: Option<Decimal>,
    max_spread: Option<Decimal>,
    to: Option<Addr>,
) -> Result<Response, ContractError> {
    offer_asset.assert_sent_native_token_balance(&info)?;

    let mut config: Config = CONFIG.load(deps.storage)?;

    // If the asset balance is already increased
    // To calculated properly we should subtract user deposit from the pool
    let pools: Vec<Asset> = config
        .pair_info
        .query_pools(&deps.querier, env.clone().contract.address)?
        .iter()
        .map(|p| {
            let mut p = p.clone();
            if p.info.equal(&offer_asset.info) {
                p.amount = p.amount.checked_sub(offer_asset.amount).unwrap();
            }

            p
        })
        .collect();

    let offer_pool: Asset;
    let ask_pool: Asset;

    if offer_asset.info.equal(&pools[0].info) {
        offer_pool = pools[0].clone();
        ask_pool = pools[1].clone();
    } else if offer_asset.info.equal(&pools[1].info) {
        offer_pool = pools[1].clone();
        ask_pool = pools[0].clone();
    } else {
        return Err(ContractError::AssetMismatch {});
    }

    // Get fee info from factory
    let fee_info = query_fee_info(
        &deps.querier,
        config.factory_addr.clone(),
        config.pair_info.pair_type.clone(),
    )?;

    let offer_amount = offer_asset.amount;
    let (return_amount, spread_amount, commission_amount) = compute_swap(
        offer_pool.amount,
        ask_pool.amount,
        offer_amount,
        fee_info.total_fee_rate,
    )?;

    // check max spread limit if exist
    assert_max_spread(
        belief_price,
        max_spread,
        offer_amount,
        return_amount + commission_amount,
        spread_amount,
    )?;

    // compute tax
    let return_asset = Asset {
        info: ask_pool.info.clone(),
        amount: return_amount,
    };

    let tax_amount = return_asset.compute_tax(&deps.querier)?;
    let receiver = to.unwrap_or_else(|| sender.clone());
    let mut messages: Vec<CosmosMsg> =
        vec![return_asset.into_msg(&deps.querier, receiver.clone())?];

    // Maker fee
    let mut maker_fee_amount = Uint128::new(0);
    if let Some(fee_address) = fee_info.fee_address {
        if let Some(f) = calculate_maker_fee(
            ask_pool.info.clone(),
            commission_amount,
            fee_info.maker_fee_rate,
        ) {
            messages.push(f.clone().into_msg(&deps.querier, fee_address)?);
            maker_fee_amount = f.amount;
        }
    }

    // Accumulate prices for oracle
    if let Some((price0_cumulative_new, price1_cumulative_new, block_time)) =
        accumulate_prices(env, &config, pools[0].amount, pools[1].amount)?
    {
        config.price0_cumulative_last = price0_cumulative_new;
        config.price1_cumulative_last = price1_cumulative_new;
        config.block_time_last = block_time;
        CONFIG.save(deps.storage, &config)?;
    }

    Ok(Response::new()
        .add_messages(
            // 1. send collateral token from the contract to a user
            // 2. send inactive commission to collector
            messages,
        )
        .add_attribute("action", "swap")
        .add_attribute("sender", sender.as_str())
        .add_attribute("receiver", receiver.as_str())
        .add_attribute("offer_asset", offer_asset.info.to_string())
        .add_attribute("ask_asset", ask_pool.info.to_string())
        .add_attribute("offer_amount", offer_amount.to_string())
        .add_attribute("return_amount", return_amount.to_string())
        .add_attribute("tax_amount", tax_amount.to_string())
        .add_attribute("spread_amount", spread_amount.to_string())
        .add_attribute("commission_amount", commission_amount.to_string())
        .add_attribute("maker_fee_amount", maker_fee_amount.to_string()))
}

/// ## Description
/// Shifts block_time when any price is zero to not fill an accumulator with a new price to that period.
/// ## Params
/// * **env** is the object of type [`Env`].
///
/// * **config** is the object of type [`Config`].
///
/// * **x** is the balance of asset[0] within a pool
///
/// * **y** is the balance of asset[1] within a pool
pub fn accumulate_prices(
    env: Env,
    config: &Config,
    x: Uint128,
    y: Uint128,
) -> StdResult<Option<(Uint128, Uint128, u64)>> {
    let block_time = env.block.time.seconds();
    if block_time <= config.block_time_last {
        return Ok(None);
    }

    // we have to shift block_time when any price is zero to not fill an accumulator with a new price to that period

    let time_elapsed = Uint128::from(block_time - config.block_time_last);

    let mut pcl0 = config.price0_cumulative_last;
    let mut pcl1 = config.price1_cumulative_last;

    if !x.is_zero() && !y.is_zero() {
        let price_precision = Uint128::from(10u128.pow(TWAP_PRECISION.into()));
        pcl0 = config.price0_cumulative_last.wrapping_add(
            time_elapsed
                .checked_mul(price_precision)?
                .multiply_ratio(y, x),
        );
        pcl1 = config.price1_cumulative_last.wrapping_add(
            time_elapsed
                .checked_mul(price_precision)?
                .multiply_ratio(x, y),
        );
    };

    Ok(Some((pcl0, pcl1, block_time)))
}

/// ## Description
/// Calculates the maker commission according to the specified parameters.
/// Returns an [`None`] if maker fee is zero, otherwise returns the [`Asset`] with the specified attributes.
/// ## Params
/// * **pool_info** is the object of type [`AssetInfo`]. Information about the pool for which the commission will be calculated.
///
/// * **commission_amount** is the object of type [`Env`]. Sets the commission amount for the pool.
///
/// * **maker_commission_rate** is the object of type [`MessageInfo`]. Sets the maker commission rate for the pool.
pub fn calculate_maker_fee(
    pool_info: AssetInfo,
    commission_amount: Uint128,
    maker_commission_rate: Decimal,
) -> Option<Asset> {
    let maker_fee: Uint128 = commission_amount * maker_commission_rate;
    if maker_fee.is_zero() {
        return None;
    }

    Some(Asset {
        info: pool_info,
        amount: maker_fee,
    })
}

/// ## Description
/// Available the query messages of the contract.
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **_env** is the object of type [`Env`].
///
/// * **msg** is the object of type [`QueryMsg`].
///
/// ## Queries
/// * **QueryMsg::Pair {}** Returns information about a pair in an object of type [`PairInfo`].
///
/// * **QueryMsg::Pool {}** Returns information about a pool in an object of type [`PoolResponse`].
///
/// * **QueryMsg::Share { amount }** Returns information about the share of the pool in a vector
/// that contains objects of type [`Asset`].
///
/// * **QueryMsg::Simulation { offer_asset }** Returns information about the simulation of the
/// swap in a [`SimulationResponse`] object.
///
/// * **QueryMsg::ReverseSimulation { ask_asset }** Returns information about the reverse simulation
/// in a [`ReverseSimulationResponse`] object.
///
/// * **QueryMsg::CumulativePrices {}** Returns information about the cumulative prices in a
/// [`CumulativePricesResponse`] object.
///
/// * **QueryMsg::Config {}** Returns information about the controls settings in a
/// [`ConfigResponse`] object.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Pair {} => to_json_binary(&query_pair_info(deps)?),
        QueryMsg::Pool {} => to_json_binary(&query_pool(deps)?),
        QueryMsg::Share { amount } => to_json_binary(&query_share(deps, amount)?),
        QueryMsg::Simulation { offer_asset } => to_json_binary(&query_simulation(deps, offer_asset)?),
        QueryMsg::ReverseSimulation { ask_asset } => {
            to_json_binary(&query_reverse_simulation(deps, ask_asset)?)
        }
        QueryMsg::CumulativePrices {} => to_json_binary(&query_cumulative_prices(deps, env)?),
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
    }
}

/// ## Description
/// Returns information about a pair in an object of type [`PairInfo`].
/// ## Params
/// * **deps** is the object of type [`Deps`].
pub fn query_pair_info(deps: Deps) -> StdResult<PairInfo> {
    let config: Config = CONFIG.load(deps.storage)?;
    Ok(config.pair_info)
}

/// ## Description
/// Returns information about a pool in an object of type [`PoolResponse`].
/// ## Params
/// * **deps** is the object of type [`Deps`].
pub fn query_pool(deps: Deps) -> StdResult<PoolResponse> {
    let config: Config = CONFIG.load(deps.storage)?;
    let (assets, total_share) = pool_info(deps, config)?;

    let resp = PoolResponse {
        assets,
        total_share,
    };

    Ok(resp)
}

/// ## Description
/// Returns information about the share of the pool in a vector that contains objects of type [`Asset`].
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **amount** is the object of type [`Uint128`]. Sets the amount for which a share in the pool will be requested.
pub fn query_share(deps: Deps, amount: Uint128) -> StdResult<Vec<Asset>> {
    let config: Config = CONFIG.load(deps.storage)?;
    let (pools, total_share) = pool_info(deps, config)?;
    let refund_assets = get_share_in_assets(&pools, amount, total_share);

    Ok(refund_assets)
}

/// ## Description
/// Returns information about the simulation of the swap in a [`SimulationResponse`] object.
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **offer_asset** is the object of type [`Asset`].
pub fn query_simulation(deps: Deps, offer_asset: Asset) -> StdResult<SimulationResponse> {
    let config: Config = CONFIG.load(deps.storage)?;
    let contract_addr = config.pair_info.contract_addr.clone();

    let pools: [Asset; 2] = config.pair_info.query_pools(&deps.querier, contract_addr)?;

    let offer_pool: Asset;
    let ask_pool: Asset;
    if offer_asset.info.equal(&pools[0].info) {
        offer_pool = pools[0].clone();
        ask_pool = pools[1].clone();
    } else if offer_asset.info.equal(&pools[1].info) {
        offer_pool = pools[1].clone();
        ask_pool = pools[0].clone();
    } else {
        return Err(StdError::generic_err(
            "Given offer asset doesn't belong to pairs",
        ));
    }

    // Get fee info from factory
    let fee_info = query_fee_info(
        &deps.querier,
        config.factory_addr,
        config.pair_info.pair_type,
    )?;

    let (return_amount, spread_amount, commission_amount) = compute_swap(
        offer_pool.amount,
        ask_pool.amount,
        offer_asset.amount,
        fee_info.total_fee_rate,
    )?;

    Ok(SimulationResponse {
        return_amount,
        spread_amount,
        commission_amount,
    })
}

/// ## Description
/// Returns information about the reverse simulation in a [`ReverseSimulationResponse`] object.
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **ask_asset** is the object of type [`Asset`].
pub fn query_reverse_simulation(
    deps: Deps,
    ask_asset: Asset,
) -> StdResult<ReverseSimulationResponse> {
    let config: Config = CONFIG.load(deps.storage)?;
    let contract_addr = config.pair_info.contract_addr.clone();

    let pools: [Asset; 2] = config.pair_info.query_pools(&deps.querier, contract_addr)?;

    let offer_pool: Asset;
    let ask_pool: Asset;
    if ask_asset.info.equal(&pools[0].info) {
        ask_pool = pools[0].clone();
        offer_pool = pools[1].clone();
    } else if ask_asset.info.equal(&pools[1].info) {
        ask_pool = pools[1].clone();
        offer_pool = pools[0].clone();
    } else {
        return Err(StdError::generic_err(
            "Given ask asset doesn't belong to pairs",
        ));
    }

    // Get fee info from factory
    let fee_info = query_fee_info(
        &deps.querier,
        config.factory_addr,
        config.pair_info.pair_type,
    )?;

    let (offer_amount, spread_amount, commission_amount) = compute_offer_amount(
        offer_pool.amount,
        ask_pool.amount,
        ask_asset.amount,
        fee_info.total_fee_rate,
    )?;

    Ok(ReverseSimulationResponse {
        offer_amount,
        spread_amount,
        commission_amount,
    })
}

/// ## Description
/// Returns information about the cumulative prices in a [`CumulativePricesResponse`] object.
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **env** is the object of type [`Env`].
pub fn query_cumulative_prices(deps: Deps, env: Env) -> StdResult<CumulativePricesResponse> {
    let config: Config = CONFIG.load(deps.storage)?;
    let (assets, total_share) = pool_info(deps, config.clone())?;

    let mut price0_cumulative_last = config.price0_cumulative_last;
    let mut price1_cumulative_last = config.price1_cumulative_last;

    if let Some((price0_cumulative_new, price1_cumulative_new, _)) =
        accumulate_prices(env, &config, assets[0].amount, assets[1].amount)?
    {
        price0_cumulative_last = price0_cumulative_new;
        price1_cumulative_last = price1_cumulative_new;
    }

    let resp = CumulativePricesResponse {
        assets,
        total_share,
        price0_cumulative_last,
        price1_cumulative_last,
    };

    Ok(resp)
}

/// ## Description
/// Returns information about the controls settings in a [`ConfigResponse`] object.
/// ## Params
/// * **deps** is the object of type [`Deps`].
pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config: Config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        block_time_last: config.block_time_last,
        params: None,
    })
}

/// ## Description
/// Returns an amount in the coin if the coin is found, otherwise returns [`zero`].
/// ## Params
/// * **coins** are an array of [`Coin`] type items. Sets the list of coins.
///
/// * **denom** is the object of type [`String`]. Sets the name of coin.
pub fn amount_of(coins: &[Coin], denom: String) -> Uint128 {
    match coins.iter().find(|x| x.denom == denom) {
        Some(coin) => coin.amount,
        None => Uint128::zero(),
    }
}

/// ## Description
/// Returns computed swap for the pool with specified parameters
/// ## Params
/// * **offer_pool** is the object of type [`Uint128`]. Sets the offer pool.
///
/// * **ask_pool** is the object of type [`Uint128`]. Sets the ask pool.
///
/// * **offer_amount** is the object of type [`Uint128`]. Sets the offer amount.
///
/// * **commission_rate** is the object of type [`Decimal`]. Sets the commission rate.
pub fn compute_swap(
    offer_pool: Uint128,
    ask_pool: Uint128,
    offer_amount: Uint128,
    commission_rate: Decimal,
) -> StdResult<(Uint128, Uint128, Uint128)> {
    let offer_pool: Uint256 = offer_pool.into();
    let ask_pool: Uint256 = ask_pool.into();
    let offer_amount: Uint256 = offer_amount.into();
    let commission_rate: Decimal256 = commission_rate.into();

    // offer => ask
    // ask_amount = (ask_pool - cp / (offer_pool + offer_amount))
    let cp: Uint256 = offer_pool * ask_pool;
    let return_amount: Uint256 = (ask_pool
        - Decimal256::from_ratio(cp, offer_pool + offer_amount).to_uint_ceil())
        * Uint256::one();

    // calculate spread & commission
    let spread_amount: Uint256 =
        (offer_amount * Decimal256::from_ratio(ask_pool, offer_pool)) - return_amount;
    let unsafe_spread_amount = Uint128::try_from(spread_amount).unwrap();
    let commission_amount: Uint256 = return_amount * commission_rate;
    let unsafe_commission_amount = Uint128::try_from(commission_amount).unwrap();

    // commission will be absorbed to pool
    let return_amount = return_amount - commission_amount;
    let unsafe_return_amount = Uint128::try_from(return_amount).unwrap();
    Ok((
        unsafe_return_amount,
        unsafe_spread_amount,
        unsafe_commission_amount,
    ))
}

/// ## Description
/// Returns computed offer amount for the pool with specified parameters.
/// ## Params
/// * **offer_pool** is the object of type [`Uint128`]. Sets the offer pool.
///
/// * **ask_pool** is the object of type [`Uint128`]. Sets the ask pool.
///
/// * **offer_amount** is the object of type [`Uint128`]. Sets the ask amount.
///
/// * **commission_rate** is the object of type [`Decimal`]. Sets the commission rate.
fn compute_offer_amount(
    offer_pool: Uint128,
    ask_pool: Uint128,
    ask_amount: Uint128,
    commission_rate: Decimal,
) -> StdResult<(Uint128, Uint128, Uint128)> {
    // ask => offer
    // offer_amount = cp / (ask_pool - ask_amount / (1 - commission_rate)) - offer_pool
    let cp = Uint256::from_uint128(offer_pool * ask_pool);
    let dec256_commission_rate = Decimal256::from(commission_rate);
    let uint256_ask_amount = Uint256::from_uint128(ask_amount);

    let one_minus_commission = Decimal256::one() - dec256_commission_rate;
    let inv_one_minus_commission = Decimal256::one() / one_minus_commission;

    let a = inv_one_minus_commission.mul(uint256_ask_amount);
    let b = Uint256::from_uint128(ask_pool).checked_sub(a).unwrap();
    
    let offer_amount = cp.multiply_ratio(
        Uint256::one(),
        b,
    )
    .checked_sub(Uint256::from_uint128(offer_pool))
    .unwrap();
    let unsafe_offer_amount = Uint128::try_from(offer_amount).unwrap();

    let before_commission_deduction = inv_one_minus_commission.mul(uint256_ask_amount);

    let spread_amount = Decimal256::from_ratio(ask_pool, offer_pool)
        .mul(Uint256::from_uint128(unsafe_offer_amount))
        .checked_sub(before_commission_deduction)
        .unwrap_or_else(|_| Uint256::zero());
    let unsafe_spread_amount = Uint128::try_from(spread_amount).unwrap();

    let commission_amount = dec256_commission_rate.mul(before_commission_deduction);
    let unsafe_commission_amount = Uint128::try_from(commission_amount).unwrap();

    Ok((unsafe_offer_amount, unsafe_spread_amount, unsafe_commission_amount))
}

/// ## Description
/// Returns an [`ContractError`] on failure, otherwise if `belief_price` and `max_spread` both are given, we compute new spread else we just use swap
/// spread to check `max_spread`.
/// ## Params
/// * **belief_price** is the object of type [`Option<Decimal>`]. Sets the belief price.
///
/// * **max_spread** is the object of type [`Option<Decimal>`]. Sets the maximum spread.
///
/// * **offer_amount** is the object of type [`Uint128`]. Sets the offer amount.
///
/// * **return_amount** is the object of type [`Uint128`]. Sets the return amount.
///
/// * **spread_amount** is the object of type [`Uint128`]. Sets the spread amount.
pub fn assert_max_spread(
    belief_price: Option<Decimal>,
    max_spread: Option<Decimal>,
    offer_amount: Uint128,
    return_amount: Uint128,
    spread_amount: Uint128,
) -> Result<(), ContractError> {
    let uint256_return_amount = Uint256::from_uint128(return_amount);

    let default_spread = Decimal::from_str(DEFAULT_SLIPPAGE)?;
    let max_allowed_spread = Decimal::from_str(MAX_ALLOWED_SLIPPAGE)?;

    let max_spread = max_spread.unwrap_or(default_spread);
    if max_spread.gt(&max_allowed_spread) {
        return Err(ContractError::AllowedSpreadAssertion {});
    }

    if let Some(belief_price) = belief_price {
        let expected_return = (Decimal256::one() / Decimal256::from(belief_price)).mul(Uint256::from_uint128(offer_amount));
        let spread_amount = expected_return
            .checked_sub(uint256_return_amount)
            .unwrap_or_else(|_| Uint256::zero());

        if uint256_return_amount < expected_return
            && Decimal256::from_ratio(spread_amount, expected_return) > Decimal256::from(max_spread)
        {
            return Err(ContractError::MaxSpreadAssertion {});
        }
    } else if Decimal::from_ratio(spread_amount, return_amount + spread_amount) > max_spread {
        return Err(ContractError::MaxSpreadAssertion {});
    }

    Ok(())
}

/// ## Description
/// Ensures each prices are not dropped as much as slippage tolerance rate.
/// Returns an [`ContractError`] on failure, otherwise returns [`Ok`].
/// ## Params
/// * **slippage_tolerance** is the object of type [`Option<Decimal>`].
///
/// * **deposits** are an array of [`Uint128`] type items.
///
/// * **pools** are an array of [`Asset`] type items.
fn assert_slippage_tolerance(
    slippage_tolerance: Option<Decimal>,
    deposits: &[Uint128; 2],
    pools: &[Asset; 2],
) -> Result<(), ContractError> {
    let default_slippage = Decimal::from_str(DEFAULT_SLIPPAGE)?;
    let max_allowed_slippage = Decimal::from_str(MAX_ALLOWED_SLIPPAGE)?;

    let slippage_tolerance = slippage_tolerance.unwrap_or(default_slippage);
    if slippage_tolerance.gt(&max_allowed_slippage) {
        return Err(ContractError::AllowedSpreadAssertion {});
    }

    let slippage_tolerance: Decimal256 = slippage_tolerance.into();
    let one_minus_slippage_tolerance = Decimal256::one() - slippage_tolerance;
    let deposits: [Uint256; 2] = [deposits[0].into(), deposits[1].into()];
    let pools: [Uint256; 2] = [pools[0].amount.into(), pools[1].amount.into()];

    // Ensure each prices are not dropped as much as slippage tolerance rate
    if Decimal256::from_ratio(deposits[0], deposits[1]) * one_minus_slippage_tolerance
        > Decimal256::from_ratio(pools[0], pools[1])
        || Decimal256::from_ratio(deposits[1], deposits[0]) * one_minus_slippage_tolerance
            > Decimal256::from_ratio(pools[1], pools[0])
    {
        return Err(ContractError::MaxSlippageAssertion {});
    }

    Ok(())
}

/// ## Description
/// Used for migration of contract. Returns the default object of type [`Response`].
/// ## Params
/// * **_deps** is the object of type [`DepsMut`].
///
/// * **_env** is the object of type [`Env`].
///
/// * **_msg** is the object of type [`MigrateMsg`].
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

/// ## Description
/// Returns information about the pool.
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **config** is the object of type [`Config`].
pub fn pool_info(deps: Deps, config: Config) -> StdResult<([Asset; 2], Uint128)> {
    let contract_addr = config.pair_info.contract_addr.clone();
    let pools: [Asset; 2] = config.pair_info.query_pools(&deps.querier, contract_addr)?;
    let total_share: Uint128 = query_supply(&deps.querier, config.pair_info.liquidity_token)?;

    Ok((pools, total_share))
}
