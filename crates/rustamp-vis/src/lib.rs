// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-vis
//!
//! Built-in spectrum analyzer and oscilloscope (spec S08), the vis window
//! host, and conversion from the core VisFrame to Winamp-style plugin data.
//!
//! Includes the clean-room projectM 4.1.8 visualization wrapper (`vis_projectm`)
//! implementing the classic Winamp 2.95 visualization plugin ABI (S09 §5).
//!
//! [`analyser`] holds the classic analyser modes - spectrum bars with
//! falling peak markers, a triggered oscilloscope and off - as Rustamp's
//! **own design**: spec S08 does not exist yet, so every constant there is
//! an own choice and no reference behaviour is claimed.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

pub mod analyser;
pub mod projectm;
