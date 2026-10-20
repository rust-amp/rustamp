// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property tests for the pure logic of the output backend.
//!
//! The unit tests next to each module pin hand-written cases: a device that
//! is found, a period that is aligned, a sample that is clamped. A property
//! test asks the other question — *does the rule hold for every input?* — and
//! it needs many inputs to answer it.
//!
//! The inputs come from a small seeded generator, so a failure is
//! reproducible: the seed is part of the failure message, and the same seed
//! draws the same case on every run and on every machine. The crate takes no
//! dependency for it.
//!
//! # The invariants
//!
//! ## The matching rule ([`crate::selection`])
//!
//! * **A blank name matches nothing** — not a device, not another blank
//!   name.
//! * **A name matches itself**: a name that is not blank is contained in
//!   itself.
//! * **Case and surrounding space do not matter**: the rule works on the
//!   trimmed, lowercased name.
//! * **The rule is a containment.** Two names that match each other are the
//!   same name; a match that goes through a third name goes all the way; a
//!   match survives a longer device name.
//! * **The resolution agrees with the rule.** Exactly one match is the
//!   device, no match is `NotFound`, several are `Ambiguous`, and the error
//!   names the devices that do exist.
//! * **A resolved plan is stable.** Resolving it again finds the same
//!   device: the plan names the device, and the device's own name matches
//!   that device alone.
//! * **The default endpoint needs no device list.**
//!
//! ## The decision table ([`crate::recovery`])
//!
//! * **The table is total.** Every fault × selection pair has an answer, and
//!   it is the documented one.
//! * **A write failure never continues on the default**: it is not a device
//!   change, so there is no other device to continue on.
//! * **Only a healthy output needs no recovery.**
//! * **The message agrees with the decision.** It says what the next start
//!   will do, and it never offers the default device to a pinned stream.
//!
//! ## The exclusive stream ([`crate::wasapi_exclusive`], Windows)
//!
//! * **The stream is opened with a format the device accepts**, and with a
//!   channel count the player can fill.
//! * **The device's own format wins** when the device accepts it.
//! * **The device's own rate and layout win** when the device refuses its
//!   own format but accepts that rate and layout with a channel count the
//!   player drives.
//! * **A format that matches the source wins** when nothing keeps the
//!   device's own rate and layout: the engine then needs no conversion.
//! * **The conversion flag agrees with the two specs.**
//! * **The period is never below the device's minimum**, and it holds a whole
//!   number of aligned frames — the frames the *device* derives from it, not
//!   the period itself.
//! * **The period holds at least as many frames as the default period did**,
//!   and at most one alignment unit more than the larger of the default and
//!   the minimum.
//! * **A float sample is copied bit for bit.**
//! * **An integer sample round-trips within the quantisation error** of its
//!   format.
//! * **A sample beyond full scale is clamped, never wrapped.**
//! * **A sample that is not a number is silence** — in the integer layouts.
//! * **The conversion is monotone**: a louder sample never becomes a quieter
//!   integer.
//! * **The 24-bit layouts carry the same value**, packed or padded.
//! * **The extra bytes of the buffer are left alone.**
//!
//! # What is not claimed
//!
//! * The **float layout is a pass-through**: a NaN that reaches it stays a
//!   NaN. The engine cleans the samples before they reach a device (a
//!   non-finite sample is silence there), and the shared-mode path writes
//!   float samples the engine has already cleaned. The integer layouts are
//!   the ones that have to decide, and they decide "silence".
//! * The **period alignment** is checked for the sample rates a device
//!   really has (up to 192 kHz). The rule rounds the period so that the
//!   device derives the aligned frame count; a rate of ten million hertz or
//!   more would derive more frames than the rounding accounts for, and no
//!   device has such a rate.
//! * The **matching rule is not symmetric** and does not have to be: it is a
//!   containment, so "Speakers" matches "Speakers (Realtek)" but not the
//!   other way round. The suite states what containment implies.

/// How many random scenarios one property runs.
const CASES: usize = 2_000;

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// A seeded xorshift64 generator.
///
/// The property suite must be reproducible from its seed (a failure has to be
/// replayable) and the crate takes no dependency, so the generator lives
/// here. It is not a quality generator: it only has to spread the drawn
/// values evenly enough for the properties to mean something.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state must not be zero.
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    /// The next 64 random bits.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform value in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// A uniform value in `[lo, hi)`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// A uniform index in `0..n`.
    fn index(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// The characters a generated name is built from.
///
/// The names are ASCII, so the case-insensitivity property is about the
/// ASCII case folding the rule really uses.
const NAME_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 _-.()";

/// The device names a real machine has.
const DEVICE_NAMES: &[&str] = &[
    "Speakers (Realtek)",
    "Headphones (USB)",
    "CABLE Output",
    "CABLE Input",
    "HDMI Output",
    "Digital Audio (S/PDIF)",
    "Speakers",
    "Microphone (2- USB Audio Device)",
];

/// A random name: a device name of a real machine, or a name drawn from the
/// character set.
fn random_name(rng: &mut Rng) -> String {
    if rng.index(4) == 0 {
        return DEVICE_NAMES[rng.index(DEVICE_NAMES.len())].to_owned();
    }
    let length = rng.index(12);
    let mut name = String::new();
    for _ in 0..length {
        name.push(NAME_CHARS[rng.index(NAME_CHARS.len())] as char);
    }
    name
}

// ---------------------------------------------------------------------------
// The matching rule and the resolution
// ---------------------------------------------------------------------------

/// The properties of the matching rule and of the resolution of a
/// configured name.
mod selection {
    use super::{CASES, DEVICE_NAMES, Rng, random_name};
    use crate::selection::{DeviceError, DeviceTarget, matches, plan, resolve};
    use rustamp_core::config::OutputMode;

    /// The seed of the selection properties.
    const SEED: u64 = 0x5E1E_C710;

    /// How many of the devices the name matches.
    fn count_matches(wanted: &str, devices: &[String]) -> usize {
        devices.iter().filter(|name| matches(wanted, name)).count()
    }

    /// The name as the rule sees it: trimmed and lowercased.
    fn normalise(name: &str) -> String {
        name.trim().to_lowercase()
    }

    /// A random list of the devices a machine has.
    fn random_devices(rng: &mut Rng) -> Vec<String> {
        let count = rng.index(5) + 1;
        (0..count).map(|_| random_name(rng)).collect()
    }

    /// A configured name that matches exactly one of the devices, and that
    /// device.
    ///
    /// The configured text differs from the device's own name in case, in
    /// surrounding space or by being a part of it — the way a hand-written
    /// configuration differs from the name the system reports.
    fn unique_match(rng: &mut Rng, devices: &[String]) -> Option<(String, String)> {
        let unique: Vec<&String> = devices
            .iter()
            .filter(|name| count_matches(name, devices) == 1)
            .collect();
        if unique.is_empty() {
            return None;
        }
        let device = unique[rng.index(unique.len())].clone();
        // The names are ASCII, so a byte index is a character index.
        let wanted = match rng.index(3) {
            0 => device.clone(),
            1 => format!("  {}  ", device.to_ascii_lowercase()),
            _ => {
                let end = (rng.index(device.len().max(1)) + 1).min(device.len());
                device[..end].to_owned()
            }
        };
        if count_matches(&wanted, devices) == 1 {
            Some((wanted, device.clone()))
        } else {
            // The device's own name matches it alone, by construction.
            Some((device.clone(), device.clone()))
        }
    }

    /// A blank name matches nothing — not a device, not another blank name.
    #[test]
    fn a_blank_name_matches_nothing() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let name = random_name(&mut rng);
            for blank in ["", " ", "\t", "  \t ", "\n", "   "] {
                assert!(
                    !matches(blank, &name),
                    "the blank name {blank:?} matched the device {name:?}"
                );
                assert!(
                    !matches(&name, blank),
                    "the device {name:?} matched the blank name {blank:?}"
                );
            }
        }
    }

    /// A name that is not blank is contained in itself.
    #[test]
    fn a_name_matches_itself() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let name = random_name(&mut rng);
            if name.trim().is_empty() {
                continue;
            }
            assert!(
                matches(&name, &name),
                "the name {name:?} does not match itself"
            );
        }
    }

    /// The case and the surrounding space of the name do not matter.
    #[test]
    fn the_case_and_the_space_do_not_matter() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let wanted = random_name(&mut rng);
            let name = random_name(&mut rng);
            assert_eq!(
                matches(&wanted, &name),
                matches(&wanted.to_ascii_lowercase(), &name.to_ascii_lowercase()),
                "the case changed the answer for {wanted:?} and {name:?}"
            );
            assert_eq!(
                matches(&wanted, &name),
                matches(wanted.trim(), name.trim()),
                "the surrounding space changed the answer for {wanted:?} and {name:?}"
            );
        }
    }

    /// The rule is a containment: it is antisymmetric and transitive, and it
    /// survives a longer device name.
    #[test]
    fn the_rule_is_a_containment() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let a = random_name(&mut rng);
            let b = random_name(&mut rng);
            let c = random_name(&mut rng);
            if matches(&a, &b) && matches(&b, &a) {
                assert_eq!(
                    normalise(&a),
                    normalise(&b),
                    "{a:?} and {b:?} match each other but are not the same name"
                );
            }
            if matches(&a, &b) && matches(&b, &c) {
                assert!(
                    matches(&a, &c),
                    "{a:?} matches {b:?}, which matches {c:?}, but {a:?} does not match {c:?}"
                );
            }
            if matches(&a, &b) {
                let longer = format!("X {b}");
                assert!(
                    matches(&a, &longer),
                    "{a:?} matches {b:?} but not {longer:?}"
                );
            }
        }
    }

    /// The resolution agrees with the matching rule, and the error names the
    /// devices that do exist.
    #[test]
    fn the_resolution_agrees_with_the_matching_rule() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let devices = random_devices(&mut rng);
            let wanted = random_name(&mut rng);
            let count = count_matches(&wanted, &devices);
            let plan = plan(Some(&wanted), OutputMode::Shared);
            if wanted.trim().is_empty() {
                assert_eq!(
                    resolve(&plan, &devices),
                    Err(DeviceError::EmptyName),
                    "the blank name {wanted:?} is not a device"
                );
                continue;
            }
            match resolve(&plan, &devices) {
                Ok(resolved) => {
                    assert_eq!(count, 1, "{wanted:?} matched {count} devices");
                    let device = devices
                        .iter()
                        .find(|name| matches(&wanted, name))
                        .expect("the device that matched");
                    assert_eq!(
                        resolved.device,
                        DeviceTarget::Named((*device).clone()),
                        "the plan names the device, not the configured text"
                    );
                    assert_eq!(resolved.mode, OutputMode::Shared, "the mode survives");
                }
                Err(DeviceError::NotFound {
                    wanted: w,
                    available,
                }) => {
                    assert_eq!(count, 0, "{wanted:?} matched {count} devices");
                    assert_eq!(w, wanted);
                    assert_eq!(available, devices, "the error names the devices");
                }
                Err(DeviceError::Ambiguous {
                    matches: many,
                    available,
                    ..
                }) => {
                    assert!(many > 1);
                    assert_eq!(count, many, "the error counts the matches");
                    assert_eq!(available, devices, "the error names the devices");
                }
                Err(error) => panic!("unexpected error: {error:?}"),
            }
        }
    }

    /// A resolved plan is stable: resolving it again finds the same device.
    #[test]
    fn the_resolved_plan_is_stable() {
        let mut rng = Rng::new(SEED);
        let mut checked = 0;
        for _ in 0..CASES {
            let devices = random_devices(&mut rng);
            let Some((wanted, device)) = unique_match(&mut rng, &devices) else {
                continue;
            };
            checked += 1;
            for mode in [OutputMode::Shared, OutputMode::Exclusive] {
                let plan = plan(Some(&wanted), mode);
                let once = resolve(&plan, &devices).expect("the device exists");
                let twice = resolve(&once, &devices).expect("the device is still there");
                assert_eq!(
                    once, twice,
                    "the plan {once:?} changed when it was resolved again"
                );
                assert_eq!(once.device, DeviceTarget::Named(device.clone()));
                assert_eq!(once.mode, mode, "the mode survives the lookup");
            }
        }
        assert!(checked > 0, "no case had a device that matched alone");
    }

    /// The default endpoint needs no device list: the plan comes back
    /// unchanged, even when the machine has no device at all.
    #[test]
    fn the_default_target_is_never_affected_by_the_device_list() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let devices = random_devices(&mut rng);
            for mode in [OutputMode::Shared, OutputMode::Exclusive] {
                let plan = plan(None, mode);
                let resolved = resolve(&plan, &devices).expect("the default resolves");
                assert_eq!(resolved, plan);
                assert_eq!(
                    resolve(&plan, &[]).expect("the default resolves"),
                    plan,
                    "a machine without an endpoint still has a default"
                );
            }
        }
    }

    /// The device names of a real machine keep the rule working: a name that
    /// is a part of a device name finds that device, and two device names
    /// that share a part are ambiguous.
    #[test]
    fn the_real_device_names_behave_as_documented() {
        let devices: Vec<String> = DEVICE_NAMES.iter().map(|name| (*name).to_owned()).collect();
        assert!(matches("Speakers", "Speakers (Realtek)"));
        assert!(matches("  speakers  ", "Speakers (Realtek)"));
        assert!(!matches("Speakers (Realtek)", "Speakers"));
        let error = resolve(&plan(Some("CABLE"), OutputMode::Shared), &devices)
            .expect_err("two devices are named like that");
        assert!(
            matches!(error, DeviceError::Ambiguous { matches: 2, .. }),
            "{error:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The decision table
// ---------------------------------------------------------------------------

/// The properties of the fault × selection decision table.
mod recovery {
    use crate::recovery::{DeviceFault, OutputSelection, Recovery, decide, message};

    /// Every fault the classifier can report, and the healthy output.
    const FAULTS: [Option<DeviceFault>; 4] = [
        None,
        Some(DeviceFault::Invalidated),
        Some(DeviceFault::Lost),
        Some(DeviceFault::WriteFailed),
    ];

    /// Every way a device can be selected.
    const SELECTIONS: [OutputSelection; 2] = [OutputSelection::Default, OutputSelection::Pinned];

    /// The decision table is total: every fault × selection pair has an
    /// answer, and the answer is the documented one.
    #[test]
    fn the_decision_table_is_total() {
        for fault in FAULTS {
            for selection in SELECTIONS {
                let expected = match (fault, selection) {
                    (None, _) => Recovery::Nothing,
                    (Some(DeviceFault::WriteFailed), _) => Recovery::Stop,
                    (
                        Some(DeviceFault::Invalidated | DeviceFault::Lost),
                        OutputSelection::Default,
                    ) => Recovery::KeepOnDefault,
                    (
                        Some(DeviceFault::Invalidated | DeviceFault::Lost),
                        OutputSelection::Pinned,
                    ) => Recovery::Stop,
                };
                assert_eq!(
                    decide(fault, selection),
                    expected,
                    "the fault {fault:?} on a {selection:?} device"
                );
                // The same pair always gets the same answer.
                assert_eq!(decide(fault, selection), decide(fault, selection));
            }
        }
    }

    /// A write failure is not a device change: the run stops, and it never
    /// continues on the default device.
    #[test]
    fn a_write_failure_never_continues_on_the_default() {
        for selection in SELECTIONS {
            let recovery = decide(Some(DeviceFault::WriteFailed), selection);
            assert_ne!(recovery, Recovery::KeepOnDefault);
            assert_eq!(recovery, Recovery::Stop);
        }
    }

    /// Only a healthy output needs no recovery.
    #[test]
    fn only_a_healthy_output_needs_no_recovery() {
        for selection in SELECTIONS {
            assert_eq!(decide(None, selection), Recovery::Nothing);
            for fault in [
                DeviceFault::Invalidated,
                DeviceFault::Lost,
                DeviceFault::WriteFailed,
            ] {
                assert_ne!(
                    decide(Some(fault), selection),
                    Recovery::Nothing,
                    "the fault {fault:?} needs a recovery"
                );
            }
        }
    }

    /// The message says what the next start will do, and it says the same
    /// thing the decision says.
    #[test]
    fn the_message_agrees_with_the_decision() {
        for fault in [
            DeviceFault::Invalidated,
            DeviceFault::Lost,
            DeviceFault::WriteFailed,
        ] {
            for selection in SELECTIONS {
                let msg = message(fault, selection, "Speakers (Realtek)");
                assert!(msg.starts_with("Speakers (Realtek): "), "{msg}");
                assert!(!msg.is_empty());
                match decide(Some(fault), selection) {
                    Recovery::KeepOnDefault => {
                        assert!(msg.contains("current default device"), "{msg}");
                    }
                    Recovery::Stop => {
                        assert!(!msg.contains("current default device"), "{msg}");
                    }
                    Recovery::Nothing => panic!("a fault never needs no recovery"),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The exclusive stream
// ---------------------------------------------------------------------------

/// The properties of the format choice, the period choice and the sample
/// conversion of the exclusive mode.
#[cfg(windows)]
mod exclusive {
    use super::{CASES, Rng};
    use crate::wasapi_exclusive::{
        DeviceFormat, MAX_CHANNELS, SampleLayout, choose_period, plan, write_samples,
    };
    use rustamp_core::audio::AudioSpec;

    /// The seed of the exclusive properties.
    const SEED: u64 = 0xE0C1_0511;

    /// The layouts the player can produce.
    const LAYOUTS: [SampleLayout; 5] = [
        SampleLayout::Float32,
        SampleLayout::Int16,
        SampleLayout::Int24Packed,
        SampleLayout::Int24Padded,
        SampleLayout::Int32,
    ];

    /// The sample rates a device really has.
    const RATES: [u32; 9] = [
        8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000, 96_000, 192_000,
    ];

    /// A random device format.
    fn random_format(rng: &mut Rng) -> DeviceFormat {
        DeviceFormat {
            sample_rate: RATES[rng.index(RATES.len())],
            channels: (rng.index(6) + 1) as u16,
            layout: LAYOUTS[rng.index(LAYOUTS.len())],
        }
    }

    /// A random list of the formats a device accepts.
    fn random_accepted(rng: &mut Rng) -> Vec<DeviceFormat> {
        let count = rng.index(5);
        (0..count).map(|_| random_format(rng)).collect()
    }

    /// The frames the audio engine derives from a period in 100 ns units.
    ///
    /// The engine truncates the division, so this is the frame count the
    /// device really uses.
    fn frames_of(period_hns: i64, sample_rate: u32) -> i64 {
        (period_hns * i64::from(sample_rate)) / 10_000_000
    }

    /// The number of frames the buffer alignment needs.
    ///
    /// The driver asks for a buffer that is a multiple of 128 bytes, so the
    /// frame count has to be a multiple of `128 / gcd(block, 128)`.
    fn align_frames(block_align: u32) -> i64 {
        let block = u64::from(block_align.max(1));
        (128 / gcd(block, 128)) as i64
    }

    /// The greatest common divisor of two numbers.
    fn gcd(a: u64, b: u64) -> u64 {
        let (mut a, mut b) = (a, b);
        while b != 0 {
            let t = b;
            b = a % b;
            a = t;
        }
        a
    }

    /// The full-scale value of a layout, in the layout's own units.
    fn full_scale(layout: SampleLayout) -> i64 {
        match layout {
            SampleLayout::Float32 => 1,
            SampleLayout::Int16 => 32_767,
            SampleLayout::Int24Packed | SampleLayout::Int24Padded => 8_388_607,
            SampleLayout::Int32 => 2_147_483_647,
        }
    }

    /// The scale of a layout: the value of full scale in the units of the
    /// layout, as a float.
    fn scale(layout: SampleLayout) -> f64 {
        full_scale(layout) as f64
    }

    /// The integer a layout holds, read back from the bytes.
    fn read_int(layout: SampleLayout, out: &[u8]) -> i64 {
        match layout {
            // The float layout holds no integer: it is a bit-exact copy.
            SampleLayout::Float32 => unreachable!("the float layout holds no integer"),
            SampleLayout::Int16 => i64::from(i16::from_le_bytes([out[0], out[1]])),
            SampleLayout::Int24Packed => {
                let raw = i32::from(out[0]) | (i32::from(out[1]) << 8) | (i32::from(out[2]) << 16);
                i64::from((raw << 8) >> 8)
            }
            SampleLayout::Int24Padded => {
                i64::from(i32::from_le_bytes([out[0], out[1], out[2], out[3]]) >> 8)
            }
            SampleLayout::Int32 => i64::from(i32::from_le_bytes([out[0], out[1], out[2], out[3]])),
        }
    }

    /// Write one sample and read it back.
    fn round_trip(layout: SampleLayout, sample: f32) -> i64 {
        let mut out = vec![0u8; layout.bytes_per_sample()];
        write_samples(&[sample], layout, &mut out);
        read_int(layout, &out)
    }

    /// A random sample: in range, out of range, or not a number at all.
    fn random_sample(rng: &mut Rng) -> f32 {
        match rng.index(10) {
            0 => 0.0,
            1 => 1.0,
            2 => -1.0,
            3 => rng.range(-1.0, 1.0),
            4 => rng.range(-2.0, 2.0),
            5 => rng.range(-1.0e6, 1.0e6),
            6 => f32::NAN,
            7 => f32::INFINITY,
            8 => f32::NEG_INFINITY,
            _ => rng.range(-1.0, 1.0) * 1.0e-30,
        }
    }

    /// The stream is opened with a format the device accepts, and with a
    /// channel count the player can fill.
    #[test]
    fn the_stream_is_opened_with_an_accepted_format() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let source = AudioSpec::new(RATES[rng.index(RATES.len())], (rng.index(2) + 1) as u16);
            let native = random_format(&mut rng);
            let accepted = random_accepted(&mut rng);
            let default_period = rng.range(1.0, 1.0e6) as i64;
            let min_period = rng.range(1.0, 1.0e6) as i64;
            let plan = plan(source, native, &accepted, default_period, min_period);
            let driveable = accepted
                .iter()
                .any(|format| (1..=MAX_CHANNELS).contains(&format.channels));
            assert_eq!(
                plan.is_some(),
                driveable,
                "the device accepts {accepted:?} and the source is {source:?}"
            );
            let Some(plan) = plan else {
                continue;
            };
            assert!(
                accepted.contains(&plan.format),
                "the format {:?} is not one the device accepts",
                plan.format
            );
            assert!(
                (1..=MAX_CHANNELS).contains(&plan.format.channels),
                "the player drives one or two channels, not {:?}",
                plan.format.channels
            );
            // The conversion flag agrees with the two specs.
            assert_eq!(
                plan.converts,
                plan.format.sample_rate != source.sample_rate
                    || plan.format.channels != source.channels,
                "the conversion flag disagrees with the specs"
            );
        }
    }

    /// The device's own format wins when the device accepts it.
    #[test]
    fn the_devices_own_format_wins_when_the_device_accepts_it() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let source = AudioSpec::new(RATES[rng.index(RATES.len())], (rng.index(2) + 1) as u16);
            let native = random_format(&mut rng);
            let accepted = random_accepted(&mut rng);
            let default_period = rng.range(1.0, 1.0e6) as i64;
            let min_period = rng.range(1.0, 1.0e6) as i64;
            let Some(plan) = plan(source, native, &accepted, default_period, min_period) else {
                continue;
            };
            if (1..=MAX_CHANNELS).contains(&native.channels) && accepted.contains(&native) {
                assert_eq!(
                    plan.format, native,
                    "the device's own format was accepted, so it wins"
                );
            }
        }
    }

    /// The device's own rate and layout win when it refuses its own format
    /// but accepts that rate and layout with a channel count the player
    /// drives.
    #[test]
    fn the_own_rate_and_layout_win_when_the_own_format_is_refused() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let source = AudioSpec::new(RATES[rng.index(RATES.len())], (rng.index(2) + 1) as u16);
            let native = random_format(&mut rng);
            let accepted = random_accepted(&mut rng);
            let default_period = rng.range(1.0, 1.0e6) as i64;
            let min_period = rng.range(1.0, 1.0e6) as i64;
            let Some(plan) = plan(source, native, &accepted, default_period, min_period) else {
                continue;
            };
            let keeps_own = accepted.iter().any(|format| {
                format.sample_rate == native.sample_rate
                    && format.layout == native.layout
                    && (1..=MAX_CHANNELS).contains(&format.channels)
            });
            let native_accepted =
                (1..=MAX_CHANNELS).contains(&native.channels) && accepted.contains(&native);
            if keeps_own && !native_accepted {
                assert_eq!(
                    plan.format.sample_rate, native.sample_rate,
                    "the device's own rate is kept"
                );
                assert_eq!(
                    plan.format.layout, native.layout,
                    "the device's own layout is kept"
                );
            }
        }
    }

    /// A format that matches the source wins when nothing keeps the device's
    /// own rate and layout: the engine then needs no conversion at all.
    #[test]
    fn a_format_that_matches_the_source_wins_when_nothing_keeps_the_own_format() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let source = AudioSpec::new(RATES[rng.index(RATES.len())], (rng.index(2) + 1) as u16);
            let native = random_format(&mut rng);
            let accepted = random_accepted(&mut rng);
            let default_period = rng.range(1.0, 1.0e6) as i64;
            let min_period = rng.range(1.0, 1.0e6) as i64;
            let Some(plan) = plan(source, native, &accepted, default_period, min_period) else {
                continue;
            };
            let keeps_own = accepted.iter().any(|format| {
                format.sample_rate == native.sample_rate
                    && format.layout == native.layout
                    && (1..=MAX_CHANNELS).contains(&format.channels)
            });
            let native_accepted =
                (1..=MAX_CHANNELS).contains(&native.channels) && accepted.contains(&native);
            let matches_source = accepted.iter().any(|format| {
                format.sample_rate == source.sample_rate && format.channels == source.channels
            });
            if !keeps_own && !native_accepted && matches_source {
                assert_eq!(
                    plan.format.sample_rate, source.sample_rate,
                    "the source's rate is kept"
                );
                assert_eq!(
                    plan.format.channels, source.channels,
                    "the source's channel count is kept"
                );
                assert!(!plan.converts, "the engine needs no conversion at all");
            }
        }
    }

    /// The same inputs always give the same plan.
    #[test]
    fn the_plan_is_deterministic() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let source = AudioSpec::new(RATES[rng.index(RATES.len())], (rng.index(2) + 1) as u16);
            let native = random_format(&mut rng);
            let accepted = random_accepted(&mut rng);
            let default_period = rng.range(1.0, 1.0e6) as i64;
            let min_period = rng.range(1.0, 1.0e6) as i64;
            let once = plan(source, native, &accepted, default_period, min_period);
            let twice = plan(source, native, &accepted, default_period, min_period);
            assert_eq!(once, twice);
        }
    }

    /// The period is never below the device's minimum, and it holds a whole
    /// number of aligned frames.
    #[test]
    fn the_period_is_never_below_the_minimum_and_is_aligned() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let rate = RATES[rng.index(RATES.len())];
            let block_align = (rng.index(16) + 1) as u32;
            let default_period = rng.range(1.0, 2.0e6) as i64;
            let min_period = rng.range(1.0, 2.0e6) as i64;
            let period = choose_period(default_period, min_period, rate, block_align);
            assert!(
                period >= min_period,
                "the period {period} is below the minimum {min_period}"
            );
            assert!(period >= 1, "the period {period} is not a period");
            let frames = frames_of(period, rate);
            let align = align_frames(block_align);
            assert_eq!(
                frames % align,
                0,
                "the period holds {frames} frames, which is not a multiple of {align}"
            );
            // The frames of the minimum, rounded up: the period holds at
            // least that many.
            let min_frames = (min_period * i64::from(rate) + 9_999_999) / 10_000_000;
            assert!(
                frames >= min_frames,
                "the period holds {frames} frames, fewer than the {min_frames} the minimum asks for"
            );
            // The period holds at least as many frames as the default period
            // did, and at most one alignment unit more than the larger of the
            // default and the minimum.
            let default_frames = frames_of(default_period, rate);
            assert!(
                frames >= default_frames,
                "the period holds {frames} frames, fewer than the {default_frames} of the default"
            );
            let bigger = default_period.max(min_period);
            assert!(
                frames <= frames_of(bigger, rate) + align,
                "the period holds {frames} frames, more than one alignment unit above the default"
            );
        }
    }

    /// A float sample is copied bit for bit.
    #[test]
    fn a_float_sample_is_copied_bit_for_bit() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let samples: Vec<f32> = (0..8).map(|_| random_sample(&mut rng)).collect();
            let mut out = vec![0u8; samples.len() * 4];
            write_samples(&samples, SampleLayout::Float32, &mut out);
            for (index, (written, chunk)) in samples
                .iter()
                .zip(out.as_chunks::<4>().0.iter())
                .enumerate()
            {
                assert_eq!(
                    written.to_bits(),
                    f32::from_le_bytes(*chunk).to_bits(),
                    "the sample {index} ({written}) changed"
                );
            }
        }
    }

    /// An integer sample round-trips within the quantisation error of its
    /// format.
    #[test]
    fn an_integer_sample_round_trips_within_the_quantisation_error() {
        let mut rng = Rng::new(SEED);
        for layout in LAYOUTS {
            if layout == SampleLayout::Float32 {
                continue;
            }
            for _ in 0..CASES {
                let sample = rng.range(-1.0, 1.0);
                let value = round_trip(layout, sample);
                let back = value as f64 / scale(layout);
                let error = (f64::from(sample) - back).abs();
                assert!(
                    error <= 0.5 / scale(layout) + 1.0e-9,
                    "the sample {sample} came back as {back} (the error is {error})"
                );
            }
        }
    }

    /// A sample beyond full scale is clamped to full scale, and it never
    /// wraps around into the other half of the range.
    #[test]
    fn a_sample_beyond_full_scale_is_clamped_and_never_wraps() {
        for layout in LAYOUTS {
            if layout == SampleLayout::Float32 {
                continue;
            }
            let full = full_scale(layout);
            for sample in [1.0f32, -1.0, 2.0, -2.0, 1.0e30, -1.0e30, f32::MAX, f32::MIN] {
                let value = round_trip(layout, sample);
                assert!(
                    (-full - 1..=full).contains(&value),
                    "the sample {sample} became {value}, which is outside the format"
                );
                if sample > 1.0 {
                    assert_eq!(
                        value, full,
                        "the sample {sample} is not clamped to full scale"
                    );
                } else if sample < -1.0 {
                    assert_eq!(
                        value,
                        -full - 1,
                        "the sample {sample} is not clamped to full scale"
                    );
                } else if sample == 1.0 {
                    assert_eq!(value, full, "full scale is full scale");
                } else {
                    assert_eq!(value, -full, "minus full scale is the mirror of full scale");
                }
            }
        }
    }

    /// A sample that is not a number is silence — it never becomes a rail
    /// and never a random integer.
    #[test]
    fn a_non_finite_sample_is_silence() {
        for layout in LAYOUTS {
            if layout == SampleLayout::Float32 {
                // The float layout is a bit-exact copy: it cannot change the
                // value, so the policy cannot apply there.
                continue;
            }
            for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(
                    round_trip(layout, sample),
                    0,
                    "the sample {sample} is not silence in {layout:?}"
                );
            }
        }
    }

    /// The conversion is monotone: a louder sample never becomes a quieter
    /// integer.
    #[test]
    fn the_conversion_is_monotone() {
        let mut rng = Rng::new(SEED);
        for layout in LAYOUTS {
            if layout == SampleLayout::Float32 {
                continue;
            }
            for _ in 0..CASES / 10 {
                let mut samples: Vec<f32> = (0..8).map(|_| rng.range(-1.0, 1.0)).collect();
                samples.sort_by(f32::total_cmp);
                let values: Vec<i64> = samples.iter().map(|s| round_trip(layout, *s)).collect();
                for (index, pair) in values.windows(2).enumerate() {
                    assert!(
                        pair[0] <= pair[1],
                        "the samples {} and {} became {pair:?}",
                        samples[index],
                        samples[index + 1]
                    );
                }
            }
        }
    }

    /// The 24-bit layouts carry the same value, packed in three bytes or
    /// padded into four.
    #[test]
    fn the_24_bit_layouts_carry_the_same_value() {
        let mut rng = Rng::new(SEED);
        for _ in 0..CASES {
            let sample = rng.range(-1.0, 1.0);
            let packed = round_trip(SampleLayout::Int24Packed, sample);
            let padded = round_trip(SampleLayout::Int24Padded, sample);
            assert_eq!(
                packed, padded,
                "the sample {sample} differs between the layouts"
            );
        }
    }

    /// The extra bytes of the buffer are left alone.
    #[test]
    fn the_extra_bytes_of_the_buffer_are_left_alone() {
        let mut rng = Rng::new(SEED);
        for layout in LAYOUTS {
            for _ in 0..CASES / 10 {
                let sample = random_sample(&mut rng);
                let width = layout.bytes_per_sample();
                let mut out = vec![0xA5u8; width * 3];
                write_samples(&[sample], layout, &mut out);
                let tail = vec![0xA5u8; width * 2];
                assert_eq!(&out[width..], &tail, "the tail of the buffer was written");
            }
        }
    }
}
