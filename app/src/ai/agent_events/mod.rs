//! Shared agent-event stream utilities used by orchestration consumers and
//! third-party harness bridges.

mod driver;
mod message_hydrator;

pub(crate) use driver::{
    run_agent_event_driver, AgentEventConsumer, AgentEventConsumerControlFlow,
    AgentEventDriverConfig, AgentEventFilter, AgentMessageEventMetadata, ServerApiAgentEventSource,
};
pub(crate) use message_hydrator::MessageHydrator;
