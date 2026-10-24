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
//! falling peak markers, a triggered oscilloscope and off. It follows the
//! audited S08 for the area and the band count, the level scale, the fall-off
//! step and the peak factor; what S08 leaves open - the band frequency edges,
//! the update rate and the direction of the peak marker's travel - is an
//! **own choice** there and says so. The same holds for the oscilloscope:
//! its sampling (S08 open question 3) and the `sa` value that selects it
//! (open question 5) are own choices, documented and pinned there.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

pub mod analyser;
pub mod projectm;
