// SPDX-License-Identifier: AGPL-3.0-only

#![allow(clippy::arithmetic_side_effects)]
#![deny(missing_docs)]

//! An Uniswap-like program for the Solana blockchain.

pub mod constraints;
pub mod curve;
pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;
pub mod redemption_rate_processor;
pub mod permission;

#[cfg(not(feature = "no-entrypoint"))]
mod entrypoint;

// Export current sdk types for downstream users building with a different sdk
// version
pub use solana_program;

solana_program::declare_id!("PSMi4nPG9aB2cKksv5LoxVS1RufH1bew59vr6bxdDYH");
