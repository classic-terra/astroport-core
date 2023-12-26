use classic_rust::types::terra::market::v1beta1::{QuerySwapResponse, QuerySwapRequest};
use classic_rust::types::cosmos::base::v1beta1::Coin as ClassicCoin;
use classic_rust::types::terra::treasury::v1beta1::{QueryTaxRateResponse, QueryTaxCapRequest, QueryTaxCapResponse};
use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_json, to_json_binary, Addr, Binary, Coin, ContractResult, Decimal, OwnedDeps,
    Querier, QuerierResult, QueryRequest, SystemError, SystemResult, Uint128, WasmQuery, Empty,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::str::FromStr;

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::PairType;
use astroport::pair::SimulationResponse;
use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Pair { asset_infos: [AssetInfo; 2] },
    Simulation { offer_asset: Asset },
}

/// mock_dependencies is a drop-in replacement for cosmwasm_std::testing::mock_dependencies
/// this uses our CustomQuerier.
pub fn mock_dependencies(
    contract_balance: &[Coin],
) -> OwnedDeps<MockStorage, MockApi, WasmMockQuerier> {
    let custom_querier: WasmMockQuerier =
        WasmMockQuerier::new(MockQuerier::new(&[(MOCK_CONTRACT_ADDR, contract_balance)]));

    OwnedDeps {
        storage: MockStorage::default(),
        api: MockApi::default(),
        querier: custom_querier,
        custom_query_type: PhantomData
    }
}

pub struct WasmMockQuerier {
    base: MockQuerier<Empty>,
    token_querier: TokenQuerier,
    tax_querier: TaxQuerier,
    astroport_factory_querier: AstroportFactoryQuerier,
}

#[derive(Clone, Default)]
pub struct TokenQuerier {
    // this lets us iterate over all pairs that match the first string
    balances: HashMap<String, HashMap<String, Uint128>>,
}

impl TokenQuerier {
    pub fn new(balances: &[(&String, &[(&String, &Uint128)])]) -> Self {
        TokenQuerier {
            balances: balances_to_map(balances),
        }
    }
}

pub(crate) fn balances_to_map(
    balances: &[(&String, &[(&String, &Uint128)])],
) -> HashMap<String, HashMap<String, Uint128>> {
    let mut balances_map: HashMap<String, HashMap<String, Uint128>> = HashMap::new();
    for (contract_addr, balances) in balances.iter() {
        let mut contract_balances_map: HashMap<String, Uint128> = HashMap::new();
        for (addr, balance) in balances.iter() {
            contract_balances_map.insert(addr.to_string(), **balance);
        }

        balances_map.insert(contract_addr.to_string(), contract_balances_map);
    }
    balances_map
}

#[derive(Clone, Default)]
pub struct TaxQuerier {
    rate: Decimal,
    // this lets us iterate over all pairs that match the first string
    caps: HashMap<String, Uint128>,
}

impl TaxQuerier {
    pub fn new(rate: Decimal, caps: &[(&String, &Uint128)]) -> Self {
        TaxQuerier {
            rate,
            caps: caps_to_map(caps),
        }
    }
}

pub(crate) fn caps_to_map(caps: &[(&String, &Uint128)]) -> HashMap<String, Uint128> {
    let mut owner_map: HashMap<String, Uint128> = HashMap::new();
    for (denom, cap) in caps.iter() {
        owner_map.insert(denom.to_string(), **cap);
    }
    owner_map
}

#[derive(Clone, Default)]
pub struct AstroportFactoryQuerier {
    pairs: HashMap<String, String>,
}

impl AstroportFactoryQuerier {
    pub fn new(pairs: &[(&String, &String)]) -> Self {
        AstroportFactoryQuerier {
            pairs: pairs_to_map(pairs),
        }
    }
}

pub(crate) fn pairs_to_map(pairs: &[(&String, &String)]) -> HashMap<String, String> {
    let mut pairs_map: HashMap<String, String> = HashMap::new();
    for (key, pair) in pairs.iter() {
        pairs_map.insert(key.to_string(), pair.to_string());
    }
    pairs_map
}

impl Querier for WasmMockQuerier {
    fn raw_query(&self, bin_request: &[u8]) -> QuerierResult {
        // MockQuerier doesn't support Custom, so we ignore it completely here
        let request: QueryRequest<Empty> = match from_json(bin_request) {
            Ok(v) => v,
            Err(e) => {
                return SystemResult::Err(SystemError::InvalidRequest {
                    error: format!("Parsing query request: {}", e),
                    request: bin_request.into(),
                })
            }
        };
        self.handle_query(&request)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MockQueryMsg {
    Price {},
}

impl WasmMockQuerier {
    pub fn handle_query(&self, request: &QueryRequest<Empty>) -> QuerierResult {
        match &request {
            QueryRequest::Stargate { path, data } => {
                match path.as_str() {
                    "/terra.treasury.v1beta1.Query/TaxRate" => {
                        let res = QueryTaxRateResponse {
                            tax_rate: self.tax_querier.rate.to_string(),
                        };
                        SystemResult::Ok(to_json_binary(&res).into())
                    }
                    "/terra.treasury.v1beta1.Query/TaxCap" => {
                        let req : QueryTaxCapRequest = Binary::try_into(data.clone()).unwrap();

                        let tax_cap = self
                            .tax_querier
                            .caps
                            .get(&req.denom)
                            .copied()
                            .unwrap_or_default()
                            .to_string();
                        let res = QueryTaxCapResponse { 
                            tax_cap 
                        };
                        SystemResult::Ok(to_json_binary(&res).into())
                    }
                    "/terra.market.v1beta1.Query/Swap" => {
                        let req : QuerySwapRequest = Binary::try_into(data.clone()).unwrap();

                        let coin = Coin::from_str(&req.offer_coin).unwrap();
                        let res = QuerySwapResponse {
                            return_coin: Some(ClassicCoin {
                                denom: coin.denom,
                                amount: coin.amount.to_string()
                            }),
                        };
                        SystemResult::Ok(ContractResult::from(to_json_binary(&res)))
                    }
                    _ => panic!("NO SUCH REQUEST")
                }
            }
            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => {
                if contract_addr.to_string().starts_with("token")
                    || contract_addr.to_string().starts_with("asset")
                {
                    self.handle_cw20(contract_addr, msg)
                } else {
                    self.handle_default(msg)
                }
            }
            _ => self.base.handle_query(request),
        }
    }

    fn handle_default(&self, msg: &Binary) -> QuerierResult {
        match from_json(&msg).unwrap() {
            QueryMsg::Pair { asset_infos } => {
                let key = asset_infos[0].to_string() + asset_infos[1].to_string().as_str();
                match self.astroport_factory_querier.pairs.get(&key) {
                    Some(v) => SystemResult::Ok(ContractResult::from(to_json_binary(&PairInfo {
                        contract_addr: Addr::unchecked(v),
                        liquidity_token: Addr::unchecked("liquidity"),
                        asset_infos: [
                            AssetInfo::NativeToken {
                                denom: "uusd".to_string(),
                            },
                            AssetInfo::NativeToken {
                                denom: "uusd".to_string(),
                            },
                        ],
                        pair_type: PairType::Xyk {},
                    }))),
                    None => SystemResult::Err(SystemError::InvalidRequest {
                        error: "No pair info exists".to_string(),
                        request: msg.as_slice().into(),
                    }),
                }
            }
            QueryMsg::Simulation { offer_asset } => {
                SystemResult::Ok(ContractResult::from(to_json_binary(&SimulationResponse {
                    return_amount: offer_asset.amount,
                    commission_amount: Uint128::zero(),
                    spread_amount: Uint128::zero(),
                })))
            }
        }
    }

    fn handle_cw20(&self, contract_addr: &String, msg: &Binary) -> QuerierResult {
        match from_json(&msg).unwrap() {
            Cw20QueryMsg::TokenInfo {} => {
                let balances: &HashMap<String, Uint128> =
                    match self.token_querier.balances.get(contract_addr) {
                        Some(balances) => balances,
                        None => {
                            return SystemResult::Err(SystemError::Unknown {});
                        }
                    };

                let mut total_supply = Uint128::zero();

                for balance in balances {
                    total_supply += *balance.1;
                }

                SystemResult::Ok(ContractResult::from(to_json_binary(&TokenInfoResponse {
                    name: "mAPPL".to_string(),
                    symbol: "mAPPL".to_string(),
                    decimals: 6,
                    total_supply: total_supply,
                })))
            }
            Cw20QueryMsg::Balance { address } => {
                let balances: &HashMap<String, Uint128> =
                    match self.token_querier.balances.get(contract_addr) {
                        Some(balances) => balances,
                        None => {
                            return SystemResult::Err(SystemError::Unknown {});
                        }
                    };

                let balance = match balances.get(&address) {
                    Some(v) => v,
                    None => {
                        return SystemResult::Err(SystemError::Unknown {});
                    }
                };

                SystemResult::Ok(ContractResult::from(to_json_binary(&BalanceResponse {
                    balance: *balance,
                })))
            }
            _ => panic!("DO NOT ENTER HERE"),
        }
    }
}

impl WasmMockQuerier {
    pub fn new(base: MockQuerier<Empty>) -> Self {
        WasmMockQuerier {
            base,
            token_querier: TokenQuerier::default(),
            tax_querier: TaxQuerier::default(),
            astroport_factory_querier: AstroportFactoryQuerier::default(),
        }
    }

    pub fn with_balance(&mut self, balances: &[(&String, &[Coin])]) {
        for (addr, balance) in balances {
            self.base.update_balance(addr.clone(), balance.to_vec());
        }
    }

    pub fn with_token_balances(&mut self, balances: &[(&String, &[(&String, &Uint128)])]) {
        self.token_querier = TokenQuerier::new(balances);
    }

    pub fn with_tax(&mut self, rate: Decimal, caps: &[(&String, &Uint128)]) {
        self.tax_querier = TaxQuerier::new(rate, caps);
    }

    pub fn with_astroport_pairs(&mut self, pairs: &[(&String, &String)]) {
        self.astroport_factory_querier = AstroportFactoryQuerier::new(pairs);
    }
}
