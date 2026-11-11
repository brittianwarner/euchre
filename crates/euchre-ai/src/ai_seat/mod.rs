//! The pure half of the `aiSeat` actor (`src/lib/actors/ai-seat/*`): its durable
//! state shapes, difficulty narrowing, per-hand memory and cross-hand notes, the
//! status helpers, and the ladder's pure rungs. The actor itself (queues, model
//! calls, sends) belongs to the LLM and server stages.

pub mod bridge;
pub mod difficulty;
pub mod handlers;
pub mod ladder;
pub mod memory;
pub mod state;
pub mod status;
