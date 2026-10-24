//! Volume and balance mapping.
//!
//! This is the **one** place where the Winamp-style control values are turned
//! into gains. Everything that needs a gain goes through here, so a later
//! change (specs S02/S07 may define a non-linear slider law and a different
//! balance law) touches this file only.
//!
//! Provisional mapping (documented for the report):
//!
//! * **Volume** `u8` `0..=255` -> linear gain `volume / 255`. `0` is silence,
//!   `255` is unity. The original player's slider is not linear, but the law is
//!   not specified yet (S02).
//! * **Balance** `i8` `-127..=127` -> `(left, right)`. `0` leaves both channels
//!   alone, `-127` mutes the right channel and `+127` the left, linearly in
//!   between: `left = 1 - b/127` for positive `b`, `right = 1 + b/127` for
//!   negative `b`. Attenuation only, never boost. Negative is left, as the core
//!   documents.
//! * **Mono** output cannot pan: the mean of the two channel gains is used.
//! * More than two channels: even indices use the left gain, odd ones the right.

/// Linear gain for a Winamp-style volume `0..=255`.
pub fn volume_gain(volume: u8) -> f32 {
    f32::from(volume) / 255.0
}

/// Left/right gains for a balance `-127..=127` (values outside are clamped).
///
/// Negative balance moves the image left, so the **right** channel is
/// attenuated; positive balance attenuates the left one. Neither ever boosts.
pub fn balance_gains(balance: i8) -> (f32, f32) {
    let x = f32::from(balance.clamp(-127, 127)) / 127.0;
    if x < 0.0 {
        (1.0, 1.0 + x)
    } else {
        (1.0 - x, 1.0)
    }
}

/// Combined per-channel gains for an interleaved buffer with `channels`
/// channels.
pub fn channel_gains(volume: u8, balance: i8, channels: u16) -> (f32, f32) {
    let v = volume_gain(volume);
    let (l, r) = balance_gains(balance);
    let (l, r) = (l * v, r * v);
    if channels <= 1 {
        ((l + r) * 0.5, (l + r) * 0.5)
    } else {
        (l, r)
    }
}

/// Apply volume and balance in place to interleaved `f32` frames.
///
/// Allocation-free and branch-light: this runs on the audio thread.
#[inline]
pub fn apply(buf: &mut [f32], channels: u16, volume: u8, balance: i8) {
    if volume == 255 && balance == 0 {
        return;
    }
    let channels = usize::from(channels.max(1));
    let (left, right) = channel_gains(volume, balance, channels as u16);
    if channels == 1 {
        if left == 1.0 {
            return;
        }
        for v in buf.iter_mut() {
            *v *= left;
        }
        return;
    }
    for frame in buf.chunks_mut(channels) {
        if let Some(v) = frame.first_mut() {
            *v *= left;
        }
        if let Some(v) = frame.get_mut(1) {
            *v *= right;
        }
        for (i, v) in frame.iter_mut().enumerate().skip(2) {
            *v *= if i % 2 == 0 { left } else { right };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_boundaries() {
        assert_eq!(volume_gain(0), 0.0);
        assert_eq!(volume_gain(255), 1.0);
        assert!((volume_gain(127) - 127.0 / 255.0).abs() < f32::EPSILON);
        assert!((volume_gain(128) - 128.0 / 255.0).abs() < f32::EPSILON);
        // 127 and 128 straddle the exact midpoint.
        assert!(volume_gain(127) < 0.5 && volume_gain(128) > 0.5);
        // monotonic
        let mut last = -1.0;
        for v in 0..=255u8 {
            let g = volume_gain(v);
            assert!(g > last, "volume {v} is not monotonic");
            last = g;
        }
    }

    #[test]
    fn balance_boundaries() {
        assert_eq!(balance_gains(0), (1.0, 1.0));
        assert_eq!(balance_gains(-127), (1.0, 0.0));
        assert_eq!(balance_gains(127), (0.0, 1.0));
        assert_eq!(balance_gains(-128), (1.0, 0.0), "clamped");
        assert_eq!(balance_gains(100), (1.0 - 100.0 / 127.0, 1.0));
        // never boosts
        for b in -127..=127i8 {
            let (l, r) = balance_gains(b);
            assert!(l <= 1.0 && r <= 1.0 && l >= 0.0 && r >= 0.0);
        }
    }

    #[test]
    fn combined_gains() {
        assert_eq!(channel_gains(255, 0, 2), (1.0, 1.0));
        assert_eq!(channel_gains(0, 0, 2), (0.0, 0.0));
        let (l, r) = channel_gains(255, -127, 2);
        assert_eq!((l, r), (1.0, 0.0));
        // mono: the mean of both sides
        let (m, _) = channel_gains(255, -127, 1);
        assert_eq!(m, 0.5);
    }

    #[test]
    fn apply_scales_stereo_and_leaves_unity_alone() {
        let mut buf = [0.5f32, 0.5, -0.5, -0.5];
        apply(&mut buf, 2, 255, 0);
        assert_eq!(buf, [0.5, 0.5, -0.5, -0.5], "unity is a no-op");

        apply(&mut buf, 2, 255, -127);
        assert_eq!(buf, [0.5, 0.0, -0.5, 0.0], "full left");

        let mut buf = [1.0f32, 1.0, 1.0, 1.0];
        apply(&mut buf, 2, 128, 0);
        assert!((buf[0] - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(buf[0], buf[2]);

        let mut buf = [1.0f32, 1.0];
        apply(&mut buf, 1, 255, -127);
        assert_eq!(buf, [0.5, 0.5], "mono uses the mean gain");
    }

    #[test]
    fn apply_handles_extra_channels() {
        let mut buf = [1.0f32; 4];
        apply(&mut buf, 4, 255, 127);
        assert_eq!(buf, [0.0, 1.0, 0.0, 1.0]);
    }
}
