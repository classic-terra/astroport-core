use astroport::asset::{
    native_asset, native_asset_info, token_asset, token_asset_info, Asset, AssetInfo, PairInfo,
    ULUNA_DENOM, UUSD_DENOM,
};
use astroport::factory::{PairConfig, PairType, UpdateAddr};
use astroport::maker::{
    AssetWithLimit, BalancesResponse, ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg,
};
use astroport::token::InstantiateMsg as TokenInstantiateMsg;
use astroport_governance::utils::EPOCH_START;
use cosmwasm_std::testing::{mock_env, MockApi, MockStorage};
use cosmwasm_std::{
    attr, to_binary, Addr, Coin, Decimal, QueryRequest, Timestamp, Uint128, Uint64, WasmQuery,
};
use cw20::{BalanceResponse, Cw20QueryMsg, MinterResponse};
use std::str::FromStr;
use classic_test_tube::{self, TerraTestApp, Wasm, SigningAccount, Module, Account};

fn store_token_code(wasm: &Wasm<TerraTestApp>, owner: &SigningAccount) -> u64 {
    let astro_token_contract = std::fs::read("../../../artifacts/astroport_token.wasm").unwrap();
    let contract = wasm.store_code(&astro_token_contract, None, owner).unwrap();
    contract.data.code_id
}

fn store_pair_code(wasm: &Wasm<TerraTestApp>, owner: &SigningAccount) -> u64 {
    let pair_contract = std::fs::read("../../../artifacts/astroport_pair.wasm").unwrap();
    let contract = wasm.store_code(&pair_contract, None, owner).unwrap();
    contract.data.code_id
}

fn store_factory_code(wasm: &Wasm<TerraTestApp>, owner: &SigningAccount) -> u64 {
    let factory_contract = std::fs::read("../../../artifacts/astroport_factory.wasm").unwrap();
    let contract = wasm.store_code(&factory_contract, None, owner).unwrap();
    contract.data.code_id
}

fn instantiate_contracts(
    wasm: &Wasm<TerraTestApp>,
    owner: &SigningAccount,
    staking: Addr,
    governance_percent: Uint64,
    max_spread: Option<Decimal>,
) -> (Addr, Addr, Addr, Addr) {
    let astro_token_code_id = store_token_code(wasm, owner);

    let msg = TokenInstantiateMsg {
        name: String::from("Astro token"),
        symbol: String::from("ASTRO"),
        decimals: 6,
        initial_balances: vec![],
        mint: Some(MinterResponse {
            minter: owner.address(),
            cap: None,
        }),
        marketing: None,
    };

    let astro_token_instance = wasm
        .instantiate(
            astro_token_code_id,
            &msg,
            Some(&owner.address()),
            Some("ASTRO"),
            &[],
            owner,
        )
        .unwrap()
        .data
        .address;

    let pair_code_id = store_pair_code(&wasm, owner);

    let factory_code_id = store_factory_code(&wasm, owner);
    let msg = astroport::factory::InstantiateMsg {
        pair_configs: vec![PairConfig {
            code_id: pair_code_id,
            pair_type: PairType::Xyk {},
            total_fee_bps: 0,
            maker_fee_bps: 0,
            is_disabled: Some(false),
        }],
        token_code_id: 1u64,
        fee_address: None,
        owner: owner.address(),
        generator_address: Some(String::from("terra1rmwsanjl4tple6k3fjtqgmaepfefdwzvr6hyff")),
        whitelist_code_id: 234u64,
    };

    let factory_instance = wasm
        .instantiate(
            factory_code_id, 
            &msg, 
            Some(&owner.address()), 
            Some("FACTORY"), 
            &[], 
            owner
        ).unwrap();
        
    let escrow_fee_distributor_contract = Box::new(ContractWrapper::new_with_empty(
        astroport_escrow_fee_distributor::contract::execute,
        astroport_escrow_fee_distributor::contract::instantiate,
        astroport_escrow_fee_distributor::contract::query,
    ));

    let escrow_fee_distributor_code_id = router.store_code(escrow_fee_distributor_contract);

    let init_msg = astroport_governance::escrow_fee_distributor::InstantiateMsg {
        owner: owner.to_string(),
        astro_token: astro_token_instance.to_string(),
        voting_escrow_addr: "voting".to_string(),
        claim_many_limit: None,
        is_claim_disabled: None,
    };

    let governance_instance = router
        .instantiate_contract(
            escrow_fee_distributor_code_id,
            owner.clone(),
            &init_msg,
            &[],
            "Astroport escrow fee distributor",
            None,
        )
        .unwrap();

    let maker_contract = Box::new(ContractWrapper::new_with_empty(
        astroport_maker::contract::execute,
        astroport_maker::contract::instantiate,
        astroport_maker::contract::query,
    ));

    let market_code_id = router.store_code(maker_contract);

    let msg = InstantiateMsg {
        owner: String::from("owner"),
        factory_contract: factory_instance.to_string(),
        staking_contract: staking.to_string(),
        // governance_contract: Option::from(governance_instance.to_string()),
        // governance_percent: Option::from(governance_percent),
        astro_token_contract: astro_token_instance.to_string(),
        max_spread,
    };
    let maker_instance = router
        .instantiate_contract(
            market_code_id,
            owner,
            &msg,
            &[],
            String::from("MAKER"),
            None,
        )
        .unwrap();

    (
        astro_token_instance,
        factory_instance,
        maker_instance,
        governance_instance,
    )
}

fn instantiate_token(router: &mut TerraApp, owner: Addr, name: String, symbol: String) -> Addr {
    let token_contract = Box::new(ContractWrapper::new_with_empty(
        astroport_token::contract::execute,
        astroport_token::contract::instantiate,
        astroport_token::contract::query,
    ));

    let token_code_id = router.store_code(token_contract);

    let msg = TokenInstantiateMsg {
        name,
        symbol: symbol.clone(),
        decimals: 6,
        initial_balances: vec![],
        mint: Some(MinterResponse {
            minter: owner.to_string(),
            cap: None,
        }),
        marketing: None,
    };

    let token_instance = router
        .instantiate_contract(
            token_code_id.clone(),
            owner.clone(),
            &msg,
            &[],
            symbol,
            None,
        )
        .unwrap();
    token_instance
}

fn mint_some_token(
    router: &mut TerraApp,
    owner: Addr,
    token_instance: Addr,
    to: Addr,
    amount: Uint128,
) {
    let msg = cw20::Cw20ExecuteMsg::Mint {
        recipient: to.to_string(),
        amount,
    };
    let res = router
        .execute_contract(owner.clone(), token_instance.clone(), &msg, &[])
        .unwrap();
    assert_eq!(res.events[1].attributes[1], attr("action", "mint"));
    assert_eq!(res.events[1].attributes[2], attr("to", to.to_string()));
    assert_eq!(res.events[1].attributes[3], attr("amount", amount));
}

fn allowance_token(
    router: &mut TerraApp,
    owner: Addr,
    spender: Addr,
    token: Addr,
    amount: Uint128,
) {
    let msg = cw20::Cw20ExecuteMsg::IncreaseAllowance {
        spender: spender.to_string(),
        amount,
        expires: None,
    };
    let res = router
        .execute_contract(owner.clone(), token.clone(), &msg, &[])
        .unwrap();
    assert_eq!(
        res.events[1].attributes[1],
        attr("action", "increase_allowance")
    );
    assert_eq!(
        res.events[1].attributes[2],
        attr("owner", owner.to_string())
    );
    assert_eq!(
        res.events[1].attributes[3],
        attr("spender", spender.to_string())
    );
    assert_eq!(res.events[1].attributes[4], attr("amount", amount));
}

fn check_balance(router: &mut TerraApp, user: Addr, token: Addr, expected_amount: Uint128) {
    let msg = Cw20QueryMsg::Balance {
        address: user.to_string(),
    };

    let res: Result<BalanceResponse, _> =
        router.wrap().query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: token.to_string(),
            msg: to_binary(&msg).unwrap(),
        }));

    let balance = res.unwrap();

    assert_eq!(balance.balance, expected_amount);
}

fn create_pair(
    mut router: &mut TerraApp,
    owner: Addr,
    user: Addr,
    factory_instance: &Addr,
    assets: [Asset; 2],
) -> PairInfo {
    for a in assets.clone() {
        match a.info {
            AssetInfo::Token { contract_addr } => {
                mint_some_token(
                    &mut router,
                    owner.clone(),
                    contract_addr.clone(),
                    user.clone(),
                    a.amount,
                );
            }

            _ => {}
        }
    }

    let asset_infos = [assets[0].info.clone(), assets[1].info.clone()];

    // Create pair in factory
    let res = router
        .execute_contract(
            owner.clone(),
            factory_instance.clone(),
            &astroport::factory::ExecuteMsg::CreatePair {
                pair_type: PairType::Xyk {},
                asset_infos: asset_infos.clone(),
                init_params: None,
            },
            &[],
        )
        .unwrap();

    assert_eq!(res.events[1].attributes[1], attr("action", "create_pair"));
    assert_eq!(
        res.events[1].attributes[2],
        attr(
            "pair",
            format!(
                "{}-{}",
                asset_infos[0].to_string(),
                asset_infos[1].to_string()
            ),
        )
    );

    // Get pair
    let pair_info: PairInfo = router
        .wrap()
        .query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: factory_instance.clone().to_string(),
            msg: to_binary(&astroport::factory::QueryMsg::Pair {
                asset_infos: asset_infos.clone(),
            })
            .unwrap(),
        }))
        .unwrap();

    let mut funds = vec![];

    for a in assets.clone() {
        match a.info {
            AssetInfo::Token { contract_addr } => {
                allowance_token(
                    &mut router,
                    user.clone(),
                    pair_info.contract_addr.clone(),
                    contract_addr.clone(),
                    a.amount.clone(),
                );
            }
            AssetInfo::NativeToken { denom } => {
                funds.push(Coin {
                    denom,
                    amount: a.amount,
                });
            }
        }
    }

    funds.sort_by(|l, r| l.denom.cmp(&r.denom));

    let user_funds: Vec<Coin> = funds
        .iter()
        .map(|c| Coin {
            denom: c.denom.clone(),
            amount: c.amount * Uint128::new(2),
        })
        .collect();

    router.init_bank_balance(&user, user_funds).unwrap();

    router
        .execute_contract(
            user.clone(),
            pair_info.contract_addr.clone(),
            &astroport::pair::ExecuteMsg::ProvideLiquidity {
                assets,
                slippage_tolerance: None,
                auto_stake: None,
                receiver: None,
            },
            &funds,
        )
        .unwrap();

    pair_info
}

#[test]
fn update_config() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);

    let (astro_token_instance, factory_instance, maker_instance, governance_instance) =
        instantiate_contracts(
            &mut router,
            owner.clone(),
            staking.clone(),
            governance_percent,
            None,
        );

    let msg = QueryMsg::Config {};
    let res: ConfigResponse = router
        .wrap()
        .query_wasm_smart(&maker_instance, &msg)
        .unwrap();

    assert_eq!(res.owner, owner);
    assert_eq!(res.astro_token_contract, astro_token_instance);
    assert_eq!(res.factory_contract, factory_instance);
    assert_eq!(res.staking_contract, staking);
    assert_eq!(res.governance_contract, Some(governance_instance));
    assert_eq!(res.governance_percent, governance_percent);
    assert_eq!(res.max_spread, Decimal::from_str("0.05").unwrap());

    let new_staking = Addr::unchecked("new_staking");
    let new_factory = Addr::unchecked("new_factory");
    let new_governance = Addr::unchecked("new_governance");
    let new_governance_percent = Uint64::new(50);
    let new_max_spread = Decimal::from_str("0.5").unwrap();

    let msg = ExecuteMsg::UpdateConfig {
        governance_percent: Some(new_governance_percent),
        governance_contract: Some(UpdateAddr::Set(new_governance.to_string())),
        staking_contract: Some(new_staking.to_string()),
        factory_contract: Some(new_factory.to_string()),
        max_spread: Some(new_max_spread),
    };

    // Assert cannot update with improper owner
    let e = router
        .execute_contract(
            Addr::unchecked("not_owner"),
            maker_instance.clone(),
            &msg,
            &[],
        )
        .unwrap_err();

    assert_eq!(e.to_string(), "Unauthorized");

    router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap();

    let msg = QueryMsg::Config {};
    let res: ConfigResponse = router
        .wrap()
        .query_wasm_smart(&maker_instance, &msg)
        .unwrap();

    assert_eq!(res.factory_contract, new_factory);
    assert_eq!(res.staking_contract, new_staking);
    assert_eq!(res.governance_percent, new_governance_percent);
    assert_eq!(res.governance_contract, Some(new_governance.clone()));
    assert_eq!(res.max_spread, new_max_spread);

    let msg = ExecuteMsg::UpdateConfig {
        governance_percent: None,
        governance_contract: Some(UpdateAddr::Remove {}),
        staking_contract: None,
        factory_contract: None,
        max_spread: None,
    };

    router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap();

    let msg = QueryMsg::Config {};
    let res: ConfigResponse = router
        .wrap()
        .query_wasm_smart(&maker_instance, &msg)
        .unwrap();
    assert_eq!(res.governance_contract, None);
}

fn test_maker_collect(
    mut router: TerraApp,
    owner: Addr,
    factory_instance: Addr,
    maker_instance: Addr,
    staking: Addr,
    governance: Addr,
    governance_percent: Uint64,
    pairs: Vec<[Asset; 2]>,
    assets: Vec<AssetWithLimit>,
    bridges: Vec<(AssetInfo, AssetInfo)>,
    mint_balances: Vec<(Addr, u128)>,
    native_balances: Vec<Coin>,
    expected_balances: Vec<Asset>,
    collected_balances: Vec<(Addr, u128)>,
) {
    let user = Addr::unchecked("user0000");

    // Create pairs
    for t in pairs {
        create_pair(
            &mut router,
            owner.clone(),
            user.clone(),
            &factory_instance,
            t,
        );
    }

    // Setup bridge to withdraw USDC via USDC -> TEST -> UUSD -> ASTRO route
    router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::UpdateBridges {
                add: Some(bridges),
                remove: None,
            },
            &[],
        )
        .unwrap();

    // enable rewards distribution
    router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::EnableRewards { blocks: 1 },
            &[],
        )
        .unwrap();

    // Mint all tokens for maker
    for t in mint_balances {
        let (token, amount) = t;
        mint_some_token(
            &mut router,
            owner.clone(),
            token.clone(),
            maker_instance.clone(),
            Uint128::from(amount),
        );

        // Check initial balance
        check_balance(
            &mut router,
            maker_instance.clone(),
            token,
            Uint128::from(amount),
        );
    }

    router
        .init_bank_balance(&maker_instance, native_balances)
        .unwrap();

    let balances_resp: BalancesResponse = router
        .wrap()
        .query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: maker_instance.to_string(),
            msg: to_binary(&QueryMsg::Balances {
                assets: expected_balances.iter().map(|a| a.info.clone()).collect(),
            })
            .unwrap(),
        }))
        .unwrap();

    for b in expected_balances {
        let found = balances_resp
            .balances
            .iter()
            .find(|n| n.info.equal(&b.info))
            .unwrap();

        assert_eq!(found, &b);
    }

    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect { assets },
            &[],
        )
        .unwrap();

    for t in collected_balances {
        let (token, amount) = t;

        // Check maker balance
        check_balance(
            &mut router,
            maker_instance.clone(),
            token.clone(),
            Uint128::zero(),
        );

        // Check balances
        let amount = Uint128::new(amount);
        let governance_amount =
            amount.multiply_ratio(Uint128::from(governance_percent), Uint128::new(100));
        let staking_amount = amount - governance_amount;

        check_balance(
            &mut router,
            governance.clone(),
            token.clone(),
            governance_amount,
        );

        check_balance(&mut router, staking.clone(), token, staking_amount);
    }
}

#[test]
fn collect_all() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);
    let max_spread = Decimal::from_str("0.5").unwrap();

    let (astro_token_instance, factory_instance, maker_instance, governance_instance) =
        instantiate_contracts(
            &mut router,
            owner.clone(),
            staking.clone(),
            governance_percent,
            Some(max_spread),
        );

    let usdc_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Usdc token".to_string(),
        "USDC".to_string(),
    );

    let test_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Test token".to_string(),
        "TEST".to_string(),
    );

    let bridge2_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Bridge 2 depth token".to_string(),
        "BRIDGE".to_string(),
    );

    let uusd_asset = String::from(UUSD_DENOM);
    let uluna_asset = String::from(ULUNA_DENOM);

    // Create pairs
    let pairs = vec![
        [
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(usdc_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
    ];

    // Specify assets to swap
    let assets = vec![
        AssetWithLimit {
            info: native_asset(uusd_asset.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(astro_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: native_asset(uluna_asset.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(usdc_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(test_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(bridge2_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
    ];

    let bridges = vec![
        (
            token_asset_info(test_token_instance.clone()),
            token_asset_info(bridge2_token_instance.clone()),
        ),
        (
            token_asset_info(usdc_token_instance.clone()),
            token_asset_info(test_token_instance.clone()),
        ),
        (
            native_asset_info(uluna_asset.clone()),
            native_asset_info(uusd_asset.clone()),
        ),
    ];

    let mint_balances = vec![
        (astro_token_instance.clone(), 10u128),
        (usdc_token_instance.clone(), 20u128),
        (test_token_instance.clone(), 30u128),
    ];

    let native_balances = vec![
        Coin {
            denom: uusd_asset.clone(),
            amount: Uint128::new(100),
        },
        Coin {
            denom: uluna_asset.clone(),
            amount: Uint128::new(110),
        },
    ];

    let expected_balances = vec![
        native_asset(uusd_asset.clone(), Uint128::new(100)),
        native_asset(uluna_asset.clone(), Uint128::new(110)),
        token_asset(astro_token_instance.clone(), Uint128::new(10)),
        token_asset(usdc_token_instance.clone(), Uint128::new(20)),
        token_asset(test_token_instance.clone(), Uint128::new(30)),
    ];

    let collected_balances = vec![
        // 218 ASTRO = 10 ASTRO +
        // 84 ASTRO (100 uusd - 15 tax -> 85 - 1 fee) +
        // 79 ASTRO (110 uluna - 0 tax -> 110 uusd - 1 fee - 16 tax -> 93 - 13 tax - 1 fee) +
        // 17 ASTRO (20 usdc -> 20 test - 1 fee -> 19 bridge - 1 fee -> 18 - 1 fee) +
        // 28 ASTRO (30 test -> 30 bridge - 1 fee -> 29 - 1 fee)
        (astro_token_instance.clone(), 218u128),
        (usdc_token_instance.clone(), 0u128),
        (test_token_instance.clone(), 0u128),
    ];

    test_maker_collect(
        router,
        owner,
        factory_instance,
        maker_instance,
        staking,
        governance_instance,
        governance_percent,
        pairs,
        assets,
        bridges,
        mint_balances,
        native_balances,
        expected_balances,
        collected_balances,
    );
}

#[test]
fn collect_default_bridges() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);
    let max_spread = Decimal::from_str("0.5").unwrap();

    let (astro_token_instance, factory_instance, maker_instance, governance_instance) =
        instantiate_contracts(
            &mut router,
            owner.clone(),
            staking.clone(),
            governance_percent,
            Some(max_spread),
        );

    let bridge_uusd_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Bridge uusd token".to_string(),
        "BRIDGE-UUSD".to_string(),
    );

    let bridge_uluna_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Bridge uluna token".to_string(),
        "BRIDGE-ULUNA".to_string(),
    );

    let uusd_asset = String::from(UUSD_DENOM);
    let uluna_asset = String::from(ULUNA_DENOM);

    // Create pairs
    let pairs = vec![
        [
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(
                bridge_uusd_token_instance.clone(),
                Uint128::from(100_000_u128),
            ),
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(
                bridge_uluna_token_instance.clone(),
                Uint128::from(100_000_u128),
            ),
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
        ],
    ];

    // Set asset to swap
    let assets = vec![
        AssetWithLimit {
            info: token_asset(bridge_uusd_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(bridge_uluna_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
    ];

    // No need bridges for this
    let bridges = vec![];

    let mint_balances = vec![
        (bridge_uusd_token_instance.clone(), 100u128),
        (bridge_uluna_token_instance.clone(), 200u128),
    ];

    let native_balances = vec![];

    let expected_balances = vec![
        token_asset(bridge_uusd_token_instance.clone(), Uint128::new(100)),
        token_asset(bridge_uluna_token_instance.clone(), Uint128::new(200)),
    ];

    let collected_balances = vec![
        // 1.
        // 100 uusd-bridge -> 99 uusd (-15 native transfer fee from swap) -> 84 uusd
        // 200 uluna-bridge -1 fee -> 199 uluna

        // 2.
        // 84 uusd (-12 native transfer fee) - 1 fee -> 71 ASTRO
        // 119 uluna -1 fee -> 198 uusd (-28 native transfer fee from swap) -> 170 uusd

        // 3.
        // 170 uusd (-25 native transfer fee) -> 145 uusd -> 144 ASTRO

        // Total: 25
        (astro_token_instance, 215u128),
        // (bridge_uusd_token_instance, 0u128),
        // (bridge_uluna_token_instance, 0u128),
    ];

    test_maker_collect(
        router,
        owner,
        factory_instance,
        maker_instance,
        staking,
        governance_instance,
        governance_percent,
        pairs,
        assets,
        bridges,
        mint_balances,
        native_balances,
        expected_balances,
        collected_balances,
    );
}

#[test]
fn collect_maxdepth_test() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let user = Addr::unchecked("user0000");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);
    let max_spread = Decimal::from_str("0.5").unwrap();

    let (astro_token_instance, factory_instance, maker_instance, _) = instantiate_contracts(
        &mut router,
        owner.clone(),
        staking.clone(),
        governance_percent,
        Some(max_spread),
    );

    let usdc_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Usdc token".to_string(),
        "USDC".to_string(),
    );

    let test_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Test token".to_string(),
        "TEST".to_string(),
    );

    let bridge2_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Bridge 2 depth token".to_string(),
        "BRIDGE".to_string(),
    );

    let uusd_asset = String::from("uusd");
    let uluna_asset = String::from("uluna");

    // Create pairs
    let mut pair_addresses = vec![];
    for t in vec![
        [
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(usdc_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(usdc_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
    ] {
        let pair_info = create_pair(
            &mut router,
            owner.clone(),
            user.clone(),
            &factory_instance,
            t,
        );

        pair_addresses.push(pair_info.contract_addr);
    }

    // Setup bridge to withdraw USDC via the USDC -> TEST -> UUSD -> ASTRO route
    let err = router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::UpdateBridges {
                add: Some(vec![
                    (
                        token_asset_info(test_token_instance.clone()),
                        token_asset_info(bridge2_token_instance.clone()),
                    ),
                    (
                        token_asset_info(usdc_token_instance.clone()),
                        token_asset_info(test_token_instance.clone()),
                    ),
                    (
                        native_asset_info(uluna_asset.clone()),
                        token_asset_info(usdc_token_instance.clone()),
                    ),
                    (
                        native_asset_info(uusd_asset.clone()),
                        native_asset_info(uluna_asset.clone()),
                    ),
                ]),
                remove: None,
            },
            &[],
        )
        .unwrap_err();

    assert_eq!(err.to_string(), "Max bridge length of 2 was reached")
}

#[test]
fn collect_err_no_swap_pair() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let user = Addr::unchecked("user0000");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(50);

    let (astro_token_instance, factory_instance, maker_instance, _) = instantiate_contracts(
        &mut router,
        owner.clone(),
        staking.clone(),
        governance_percent,
        None,
    );

    let uusd_asset = String::from("uusd");
    let uluna_asset = String::from("uluna");
    let ukrt_asset = String::from("ukrt");
    let uabc_asset = String::from("uabc");

    // Mint all tokens for Maker
    for t in vec![
        [
            native_asset(ukrt_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(ukrt_asset.clone(), Uint128::from(100_000_u128)),
            native_asset(uabc_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
    ] {
        create_pair(
            &mut router,
            owner.clone(),
            user.clone(),
            &factory_instance,
            t,
        );
    }

    // Set the assets to swap
    let assets = vec![
        AssetWithLimit {
            info: native_asset(ukrt_asset.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(astro_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: native_asset(uabc_asset.clone(), Uint128::zero()).info,
            limit: None,
        },
    ];

    // Mint all tokens for the Maker
    for t in vec![(astro_token_instance.clone(), 10u128)] {
        let (token, amount) = t;
        mint_some_token(
            &mut router,
            owner.clone(),
            token.clone(),
            maker_instance.clone(),
            Uint128::from(amount),
        );

        // Check initial balance
        check_balance(
            &mut router,
            maker_instance.clone(),
            token,
            Uint128::from(amount),
        );
    }

    router
        .init_bank_balance(
            &maker_instance,
            vec![
                Coin {
                    denom: ukrt_asset,
                    amount: Uint128::new(20),
                },
                Coin {
                    denom: uabc_asset,
                    amount: Uint128::new(30),
                },
            ],
        )
        .unwrap();

    let msg = ExecuteMsg::Collect { assets };

    let e = router
        .execute_contract(maker_instance.clone(), maker_instance.clone(), &msg, &[])
        .unwrap_err();

    assert_eq!(e.to_string(), "Cannot swap uabc. No swap destinations",);
}

#[test]
fn update_bridges() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);
    let user = Addr::unchecked("user0000");
    let uusd_asset = String::from("uusd");

    let (astro_token_instance, factory_instance, maker_instance, _) = instantiate_contracts(
        &mut router,
        owner.clone(),
        staking.clone(),
        governance_percent,
        None,
    );

    let msg = ExecuteMsg::UpdateBridges {
        add: Some(vec![
            (
                native_asset_info(String::from("uluna")),
                native_asset_info(String::from("uusd")),
            ),
            (
                native_asset_info(String::from("ukrt")),
                native_asset_info(String::from("uusd")),
            ),
        ]),
        remove: None,
    };

    // Unauthorized check
    let err = router
        .execute_contract(maker_instance.clone(), maker_instance.clone(), &msg, &[])
        .unwrap_err();
    assert_eq!(err.to_string(), "Unauthorized");

    // Add bridges
    let err = router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Invalid bridge. Pool uluna to uusd not found"
    );

    // Create pair so that add bridge check does not fail
    for pair in vec![
        [
            native_asset(String::from("uluna"), Uint128::from(100_000_u128)),
            native_asset(String::from("uusd"), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(String::from("ukrt"), Uint128::from(100_000_u128)),
            native_asset(String::from("uusd"), Uint128::from(100_000_u128)),
        ],
    ] {
        create_pair(
            &mut router,
            owner.clone(),
            user.clone(),
            &factory_instance,
            pair,
        );
    }

    // Add bridges
    let err = router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Invalid bridge destination. uluna cannot be swapped to ASTRO"
    );

    // Create pair so that add bridge check does not fail
    create_pair(
        &mut router,
        owner.clone(),
        user.clone(),
        &factory_instance,
        [
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
    );

    // Add bridges
    router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap();

    let resp: Vec<(String, String)> = router
        .wrap()
        .query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: maker_instance.to_string(),
            msg: to_binary(&QueryMsg::Bridges {}).unwrap(),
        }))
        .unwrap();

    assert_eq!(
        resp,
        vec![
            (String::from("ukrt"), String::from("uusd")),
            (String::from("uluna"), String::from("uusd")),
        ]
    );

    let msg = ExecuteMsg::UpdateBridges {
        remove: Some(vec![native_asset_info(String::from("UKRT"))]),
        add: None,
    };

    // Try to remove bridges
    let err = router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Generic error: Address UKRT should be lowercase"
    );

    let msg = ExecuteMsg::UpdateBridges {
        remove: Some(vec![native_asset_info(String::from("ukrt"))]),
        add: None,
    };

    // Remove bridges
    router
        .execute_contract(owner.clone(), maker_instance.clone(), &msg, &[])
        .unwrap();

    let resp: Vec<(String, String)> = router
        .wrap()
        .query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: maker_instance.to_string(),
            msg: to_binary(&QueryMsg::Bridges {}).unwrap(),
        }))
        .unwrap();

    assert_eq!(resp, vec![(String::from("uluna"), String::from("uusd")),]);
}

#[test]
fn collect_with_asset_limit() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let user = Addr::unchecked("user0000");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);
    let max_spread = Decimal::from_str("0.5").unwrap();

    let (astro_token_instance, factory_instance, maker_instance, governance_instance) =
        instantiate_contracts(
            &mut router,
            owner.clone(),
            staking.clone(),
            governance_percent,
            Some(max_spread),
        );

    let usdc_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Usdc token".to_string(),
        "USDC".to_string(),
    );

    let test_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Test token".to_string(),
        "TEST".to_string(),
    );

    let bridge2_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Bridge 2 depth token".to_string(),
        "BRIDGE".to_string(),
    );

    let uusd_asset = String::from("uusd");
    let uluna_asset = String::from("uluna");

    // Create pairs
    for t in vec![
        [
            token_asset(usdc_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uluna_asset, Uint128::from(100_000_u128)),
            native_asset(uusd_asset, Uint128::from(100_000_u128)),
        ],
        [
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
    ] {
        create_pair(
            &mut router,
            owner.clone(),
            user.clone(),
            &factory_instance,
            t,
        );
    }

    // Make a list with duplicate assets
    let assets_with_duplicate = vec![
        AssetWithLimit {
            info: token_asset(usdc_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(usdc_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
    ];

    // Set assets to swap
    let assets = vec![
        AssetWithLimit {
            info: token_asset(astro_token_instance.clone(), Uint128::zero()).info,
            limit: Option::from(Uint128::new(5)),
        },
        AssetWithLimit {
            info: token_asset(usdc_token_instance.clone(), Uint128::zero()).info,
            limit: Option::from(Uint128::new(5)),
        },
        AssetWithLimit {
            info: token_asset(test_token_instance.clone(), Uint128::zero()).info,
            limit: Option::from(Uint128::new(5)),
        },
        AssetWithLimit {
            info: token_asset(bridge2_token_instance.clone(), Uint128::zero()).info,
            limit: Option::from(Uint128::new(5)),
        },
    ];

    // Setup bridge to withdraw USDC via the USDC -> TEST -> UUSD -> ASTRO route
    router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::UpdateBridges {
                add: Some(vec![
                    (
                        token_asset_info(test_token_instance.clone()),
                        token_asset_info(bridge2_token_instance.clone()),
                    ),
                    (
                        token_asset_info(usdc_token_instance.clone()),
                        token_asset_info(test_token_instance.clone()),
                    ),
                ]),
                remove: None,
            },
            &[],
        )
        .unwrap();

    // Enable rewards distribution
    router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::EnableRewards { blocks: 1 },
            &[],
        )
        .unwrap();

    // Mint all tokens for Maker
    for t in vec![
        (astro_token_instance.clone(), 10u128),
        (usdc_token_instance.clone(), 20u128),
        (test_token_instance.clone(), 30u128),
    ] {
        let (token, amount) = t;
        mint_some_token(
            &mut router,
            owner.clone(),
            token.clone(),
            maker_instance.clone(),
            Uint128::from(amount),
        );

        // Check initial balance
        check_balance(
            &mut router,
            maker_instance.clone(),
            token,
            Uint128::from(amount),
        );
    }

    let expected_balances = vec![
        token_asset(astro_token_instance.clone(), Uint128::new(10)),
        token_asset(usdc_token_instance.clone(), Uint128::new(20)),
        token_asset(test_token_instance.clone(), Uint128::new(30)),
    ];

    let balances_resp: BalancesResponse = router
        .wrap()
        .query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: maker_instance.to_string(),
            msg: to_binary(&QueryMsg::Balances {
                assets: expected_balances.iter().map(|a| a.info.clone()).collect(),
            })
            .unwrap(),
        }))
        .unwrap();

    for b in expected_balances {
        let found = balances_resp
            .balances
            .iter()
            .find(|n| n.info.equal(&b.info))
            .unwrap();

        assert_eq!(found, &b);
    }

    let resp = router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets_with_duplicate.clone(),
            },
            &[],
        )
        .unwrap_err();
    assert_eq!(resp.to_string(), "Cannot collect. Remove duplicate asset",);

    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    // Check Maker's balance of ASTRO tokens
    check_balance(
        &mut router,
        maker_instance.clone(),
        astro_token_instance.clone(),
        Uint128::zero(),
    );

    // Check Maker's balance of USDC tokens
    check_balance(
        &mut router,
        maker_instance.clone(),
        usdc_token_instance.clone(),
        Uint128::new(15u128),
    );

    // Check Maker's balance of test tokens
    check_balance(
        &mut router,
        maker_instance.clone(),
        test_token_instance.clone(),
        Uint128::new(0u128),
    );

    // Check balances
    // We are losing 1 ASTRO in fees per swap
    // 40 ASTRO = 10 astro +
    // 2 usdc (5 - fee for 3 swaps)
    // 28 test (30 - fee for 2 swaps)
    let amount = Uint128::new(40u128);
    let governance_amount =
        amount.multiply_ratio(Uint128::from(governance_percent), Uint128::new(100));
    let staking_amount = amount - governance_amount;

    // Check the governance contract's balance for the ASTRO token
    check_balance(
        &mut router,
        governance_instance.clone(),
        astro_token_instance.clone(),
        governance_amount,
    );

    // Check the governance contract's balance for the USDC token
    check_balance(
        &mut router,
        governance_instance.clone(),
        usdc_token_instance.clone(),
        Uint128::zero(),
    );

    // Check the governance contract's balance for the test token
    check_balance(
        &mut router,
        governance_instance.clone(),
        test_token_instance.clone(),
        Uint128::zero(),
    );

    // Check the staking contract's balance for the ASTRO token
    check_balance(
        &mut router,
        staking.clone(),
        astro_token_instance.clone(),
        staking_amount,
    );

    // Check the staking contract's balance for the USDC token
    check_balance(
        &mut router,
        staking.clone(),
        usdc_token_instance.clone(),
        Uint128::zero(),
    );

    // Check the staking contract's balance for the test token
    check_balance(
        &mut router,
        staking.clone(),
        test_token_instance.clone(),
        Uint128::zero(),
    );
}

struct CheckDistributedAstro {
    maker_amount: Uint128,
    governance_amount: Uint128,
    staking_amount: Uint128,
    governance_percent: Uint64,
    maker: Addr,
    astro_token: Addr,
    governance: Addr,
    staking: Addr,
}

impl CheckDistributedAstro {
    fn check(&mut self, router: &mut TerraApp, distributed_amount: u32) {
        let distributed_amount = Uint128::from(distributed_amount as u128);
        let cur_governance_amount = distributed_amount
            .multiply_ratio(Uint128::from(self.governance_percent), Uint128::new(100));
        self.governance_amount += cur_governance_amount;
        self.staking_amount += distributed_amount - cur_governance_amount;
        self.maker_amount -= distributed_amount;

        check_balance(
            router,
            self.maker.clone(),
            self.astro_token.clone(),
            self.maker_amount,
        );

        check_balance(
            router,
            self.governance.clone(),
            self.astro_token.clone(),
            self.governance_amount,
        );

        check_balance(
            router,
            self.staking.clone(),
            self.astro_token.clone(),
            self.staking_amount,
        );
    }
}

#[test]
fn distribute_initially_accrued_fees() {
    let mut router = mock_app();
    let owner = Addr::unchecked("owner");
    let staking = Addr::unchecked("staking");
    let governance_percent = Uint64::new(10);
    let user = Addr::unchecked("user0000");

    let (astro_token_instance, factory_instance, maker_instance, governance_instance) =
        instantiate_contracts(
            &mut router,
            owner.clone(),
            staking.clone(),
            governance_percent,
            None,
        );

    let usdc_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Usdc token".to_string(),
        "USDC".to_string(),
    );

    let test_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Test token".to_string(),
        "TEST".to_string(),
    );

    let bridge2_token_instance = instantiate_token(
        &mut router,
        owner.clone(),
        "Bridge 2 depth token".to_string(),
        "BRIDGE".to_string(),
    );

    let uusd_asset = String::from("uusd");
    let uluna_asset = String::from("uluna");

    // Create pairs
    for t in vec![
        [
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            native_asset(uluna_asset.clone(), Uint128::from(100_000_u128)),
            native_asset(uusd_asset.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(usdc_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(test_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
        [
            token_asset(bridge2_token_instance.clone(), Uint128::from(100_000_u128)),
            token_asset(astro_token_instance.clone(), Uint128::from(100_000_u128)),
        ],
    ] {
        create_pair(
            &mut router,
            owner.clone(),
            user.clone(),
            &factory_instance,
            t,
        );
    }

    // Set assets to swap
    let assets = vec![
        AssetWithLimit {
            info: native_asset(uusd_asset.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(astro_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: native_asset(uluna_asset.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(usdc_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(test_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
        AssetWithLimit {
            info: token_asset(bridge2_token_instance.clone(), Uint128::zero()).info,
            limit: None,
        },
    ];

    // Setup bridge to withdraw USDC via the USDC -> TEST -> UUSD -> ASTRO route
    router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::UpdateBridges {
                add: Some(vec![
                    (
                        token_asset_info(test_token_instance.clone()),
                        token_asset_info(bridge2_token_instance.clone()),
                    ),
                    (
                        token_asset_info(usdc_token_instance.clone()),
                        token_asset_info(test_token_instance.clone()),
                    ),
                    (
                        native_asset_info(uluna_asset.clone()),
                        native_asset_info(uusd_asset.clone()),
                    ),
                ]),
                remove: None,
            },
            &[],
        )
        .unwrap();

    // Mint all tokens for Maker
    for t in vec![
        (astro_token_instance.clone(), 10u128),
        (usdc_token_instance, 20u128),
        (test_token_instance, 30u128),
    ] {
        let (token, amount) = t;
        mint_some_token(
            &mut router,
            owner.clone(),
            token.clone(),
            maker_instance.clone(),
            Uint128::from(amount),
        );

        // Check initial balance
        check_balance(
            &mut router,
            maker_instance.clone(),
            token,
            Uint128::from(amount),
        );
    }

    router
        .init_bank_balance(
            &maker_instance,
            vec![
                Coin {
                    denom: uusd_asset,
                    amount: Uint128::new(100),
                },
                Coin {
                    denom: uluna_asset,
                    amount: Uint128::new(110),
                },
            ],
        )
        .unwrap();

    // Unauthorized check
    let err = router
        .execute_contract(
            user.clone(),
            maker_instance.clone(),
            &ExecuteMsg::EnableRewards { blocks: 1 },
            &[],
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "Unauthorized");

    // Check pre_update_blocks = 0
    let err = router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::EnableRewards { blocks: 0 },
            &[],
        )
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Generic error: Number of blocks should be > 0"
    );

    // Check that collect does not distribute ASTRO until rewards are enabled
    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect { assets },
            &[],
        )
        .unwrap();

    // Balances checker
    let mut checker = CheckDistributedAstro {
        maker_amount: Uint128::new(218_u128),
        governance_amount: Uint128::zero(),
        staking_amount: Uint128::zero(),
        maker: maker_instance.clone(),
        astro_token: astro_token_instance.clone(),
        governance_percent,
        governance: governance_instance,
        staking,
    };
    checker.check(&mut router, 0);

    // Enable rewards distribution
    router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::EnableRewards { blocks: 10 },
            &[],
        )
        .unwrap();

    // Try to enable again
    let err = router
        .execute_contract(
            owner.clone(),
            maker_instance.clone(),
            &ExecuteMsg::EnableRewards { blocks: 1 },
            &[],
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "Rewards collecting is already enabled");

    let astro_asset = AssetWithLimit {
        info: token_asset_info(astro_token_instance.clone()),
        limit: None,
    };
    let assets = vec![astro_asset];

    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    // Since the block number is the same, nothing happened
    checker.check(&mut router, 0);

    router.update_block(next_block);

    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    checker.check(&mut router, 21);

    // Let's try to collect again within the same block
    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    // But no ASTRO were distributed
    checker.check(&mut router, 0);

    router.update_block(next_block);

    // Imagine that we received new fees the while pre-ugrade ASTRO is being distributed
    mint_some_token(
        &mut router,
        owner.clone(),
        astro_token_instance.clone(),
        maker_instance.clone(),
        Uint128::from(30_u128),
    );

    let resp = router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    checker.maker_amount += Uint128::from(30_u128);
    // 51 = 30 minted astro + 21 distributed astro
    checker.check(&mut router, 51);

    // Checking that attributes are set properly
    for (attr, value) in [
        ("astro_distribution", 30_u128),
        ("preupgrade_astro_distribution", 21_u128),
    ] {
        let a = resp.events[1]
            .attributes
            .iter()
            .find(|a| a.key == attr)
            .unwrap();
        assert_eq!(a.value, value.to_string());
    }

    // Increment 8 blocks
    for _ in 0..8 {
        router.update_block(next_block);
    }

    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    // 168 = 21 * 8
    checker.check(&mut router, 168);

    // Check remainder reward
    let res: ConfigResponse = router
        .wrap()
        .query_wasm_smart(&maker_instance, &QueryMsg::Config {})
        .unwrap();

    assert_eq!(res.remainder_reward.u128(), 8_u128);

    // Check remainder reward distribution
    router.update_block(next_block);

    router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect {
                assets: assets.clone(),
            },
            &[],
        )
        .unwrap();

    checker.check(&mut router, 8);

    // Check that the pre-upgrade ASTRO was fully distributed
    let res: ConfigResponse = router
        .wrap()
        .query_wasm_smart(&maker_instance, &QueryMsg::Config {})
        .unwrap();

    assert_eq!(res.remainder_reward.u128(), 0_u128);
    assert_eq!(res.pre_upgrade_astro_amount.u128(), 218_u128);

    // Check usual collecting works
    mint_some_token(
        &mut router,
        owner,
        astro_token_instance,
        maker_instance.clone(),
        Uint128::from(115_u128),
    );

    let resp = router
        .execute_contract(
            Addr::unchecked("anyone"),
            maker_instance.clone(),
            &ExecuteMsg::Collect { assets },
            &[],
        )
        .unwrap();

    checker.maker_amount += Uint128::from(115_u128);
    checker.check(&mut router, 115);

    // Check that attributes are set properly
    let a = resp.events[1]
        .attributes
        .iter()
        .find(|a| a.key == "astro_distribution")
        .unwrap();
    assert_eq!(a.value, 115_u128.to_string());
    assert!(!resp.events[1]
        .attributes
        .iter()
        .any(|a| a.key == "preupgrade_astro_distribution"));
}
