use crate::error::ContractError;
use crate::helpers::send_tokens;
use crate::state::{
    Agent, AgentStatus, Config, GenericBalance, AGENTS, AGENTS_ACTIVE_QUEUE, AGENTS_PENDING_QUEUE,
    CONFIG,
};
use cosmwasm_std::{
    has_coins, Addr, Coin, Deps, DepsMut, Env, MessageInfo, Response, StdResult, SubMsg,
};
use cw20::Balance;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetAgentResponse {
    pub status: AgentStatus,
    pub payable_account_id: Addr,
    pub balance: GenericBalance,
    pub total_tasks_executed: u64,
    pub last_missed_slot: u64,
    pub register_start: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetAgentIdsResponse(Vec<Addr>, Vec<Addr>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetAgentTasksResponse(u64, u128);

/// Get a single agent details
/// Check's status as well, in case this agent needs to be considered for election
pub(crate) fn query_get_agent(deps: Deps, account_id: Addr) -> StdResult<Option<GetAgentResponse>> {
    let agent = AGENTS.may_load(deps.storage, account_id.clone())?;
    if agent.is_none() {
        return Ok(None);
    }
    let a = agent.unwrap();

    let pending = AGENTS_PENDING_QUEUE.load(deps.storage)?;

    // If agent is pending, Check if they should get nominated to checkin to become active
    let agent_status: AgentStatus = if a.status == AgentStatus::Pending {
        // TODO: change to check total tasks + task ratio
        if pending.contains(&account_id) {
            AgentStatus::Nominated
        } else {
            a.status
        }
    } else {
        a.status
    };

    Ok(Some(GetAgentResponse {
        status: agent_status,
        payable_account_id: a.payable_account_id,
        balance: a.balance,
        total_tasks_executed: a.total_tasks_executed,
        last_missed_slot: a.last_missed_slot,
        register_start: a.register_start,
    }))
}

/// Get a list of agent addresses
pub(crate) fn query_get_agent_ids(deps: Deps) -> StdResult<GetAgentIdsResponse> {
    let active = AGENTS_ACTIVE_QUEUE.load(deps.storage)?;
    let pending = AGENTS_PENDING_QUEUE.load(deps.storage)?;

    Ok(GetAgentIdsResponse(active, pending))
}

// TODO:
/// Check how many tasks an agent can execute
pub(crate) fn query_get_agent_tasks(
    _deps: Deps,
    _account_id: Addr,
) -> StdResult<GetAgentTasksResponse> {
    // let active = AGENTS_ACTIVE_QUEUE.load(deps.storage)?;

    Ok(GetAgentTasksResponse(0, 0))
}

/// Add any account as an agent that will be able to execute tasks.
/// Registering allows for rewards accruing with micro-payments which will accumulate to more long-term.
///
/// Optional Parameters:
/// "payable_account_id" - Allows a different account id to be specified, so a user can receive funds at a different account than the agent account.
pub fn register_agent(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    payable_account_id: Option<Addr>,
) -> Result<Response, ContractError> {
    if !info.funds.is_empty() {
        return Err(ContractError::CustomError {
            val: "Do not attach funds".to_string(),
        });
    }
    let c: Config = CONFIG.load(deps.storage)?;
    if c.paused {
        return Err(ContractError::CustomError {
            val: "Register agent paused".to_string(),
        });
    }

    let account = info.sender;

    // REF: https://github.com/CosmWasm/cw-tokens/tree/main/contracts/cw20-escrow
    // Check if native token balance is sufficient for a few txns, in this case 4 txns
    // TODO: Adjust gas & costs based on real usage cost
    let agent_wallet_balances = deps.querier.query_all_balances(account.clone())?;
    let unit_cost = c.gas_price * 4;
    if has_coins(
        &agent_wallet_balances,
        &Coin::new(u128::from(unit_cost), c.native_denom),
    ) {
        return Err(ContractError::CustomError {
            val: "Insufficient deposit".to_string(),
        });
    }

    let payable_id = payable_account_id.unwrap_or_else(|| account.clone());
    let push_account = |mut aq: Vec<Addr>| -> Result<_, ContractError> {
        aq.push(account.clone());
        Ok(aq)
    };

    let active_agents = AGENTS_ACTIVE_QUEUE.load(deps.storage)?;
    let total_agents = active_agents.len();
    let agent_status = if total_agents == 0 {
        AGENTS_ACTIVE_QUEUE.update(deps.storage, push_account)?;
        AgentStatus::Active
    } else {
        AGENTS_PENDING_QUEUE.update(deps.storage, push_account)?;
        AgentStatus::Pending
    };

    AGENTS.update(
        deps.storage,
        account,
        |a: Option<Agent>| -> Result<_, ContractError> {
            match a {
                // make sure that account isn't already added
                Some(_) => Err(ContractError::CustomError {
                    val: "Agent already exists".to_string(),
                }),
                None => {
                    Ok(Agent {
                        status: agent_status.clone(),
                        payable_account_id: payable_id,
                        balance: GenericBalance::default(),
                        total_tasks_executed: 0,
                        last_missed_slot: 0,
                        // REF: https://github.com/CosmWasm/cosmwasm/blob/main/packages/std/src/types.rs#L57
                        register_start: env.block.time.nanos(),
                    })
                }
            }
        },
    )?;

    Ok(Response::new()
        .add_attribute("method", "register_agent")
        .add_attribute("agent_status", format!("{:?}", agent_status))
        .add_attribute("register_start", env.block.time.nanos().to_string()))
}

/// Update agent details, specifically the payable account id for an agent.
pub fn update_agent(
    deps: DepsMut,
    info: MessageInfo,
    _env: Env,
    payable_account_id: Addr,
) -> Result<Response, ContractError> {
    let c: Config = CONFIG.load(deps.storage)?;
    if c.paused {
        return Err(ContractError::CustomError {
            val: "Register agent paused".to_string(),
        });
    }

    AGENTS.update(
        deps.storage,
        info.sender,
        |a: Option<Agent>| -> Result<_, ContractError> {
            match a {
                Some(agent) => {
                    let mut ag = agent;
                    ag.payable_account_id = payable_account_id;
                    Ok(ag)
                }
                None => Err(ContractError::CustomError {
                    val: "Agent doesnt exist".to_string(),
                }),
            }
        },
    )?;

    Ok(Response::new().add_attribute("method", "update_agent"))
}

/// Allows an agent to withdraw all rewards, paid to the specified payable account id.
pub(crate) fn withdraw_balances(
    deps: DepsMut,
    info: MessageInfo,
) -> Result<Vec<SubMsg>, ContractError> {
    let a = AGENTS.may_load(deps.storage, info.sender)?;
    if a.is_none() {
        return Err(ContractError::CustomError {
            val: "Agent doesnt exist".to_string(),
        });
    }
    let agent = a.unwrap();

    // This will send all token balances to Agent
    let (messages, balances) = send_tokens(&agent.payable_account_id, &agent.balance)?;
    let mut config = CONFIG.load(deps.storage)?;
    config
        .available_balance
        .minus_tokens(Balance::from(balances.native));
    // TODO: Finish:
    // config
    //     .available_balance
    //     .minus_tokens(Balance::from(balances.cw20));
    CONFIG.save(deps.storage, &config)?;

    Ok(messages)
}

/// Allows an agent to withdraw all rewards, paid to the specified payable account id.
pub fn withdraw_task_balance(
    deps: DepsMut,
    info: MessageInfo,
    _env: Env,
) -> Result<Response, ContractError> {
    let messages = withdraw_balances(deps, info.clone())?;

    Ok(Response::new()
        .add_attribute("method", "withdraw_task_balance")
        .add_attribute("account_id", info.sender)
        .add_submessages(messages))
}

/// Allows an agent to accept a nomination within a certain amount of time to become an active agent.
pub fn accept_nomination_agent(
    _deps: DepsMut,
    _info: MessageInfo,
    _env: Env,
) -> Result<Response, ContractError> {
    Ok(Response::new().add_attribute("method", "accept_nomination_agent"))
}

/// Removes the agent from the active set of agents.
/// Withdraws all reward balances to the agent payable account id.
pub fn unregister_agent(
    deps: DepsMut,
    info: MessageInfo,
    _env: Env,
) -> Result<Response, ContractError> {
    // TODO: Finish
    // let messages = withdraw_balances(deps.storage, info.clone())?;
    AGENTS.remove(deps.storage, info.sender.clone());

    Ok(Response::new()
        .add_attribute("method", "unregister_agent")
        .add_attribute("account_id", info.sender))
    // .add_submessages(messages))
}
