//! Windows named mapping/wake validation in one process. No windows or audio.

#![cfg(windows)]

use std::time::Duration;

use rustamp_ipc::Message;
use rustamp_ipc::shm::{RingDirection, RingError};
use rustamp_mediahost::MediaHostError;
use rustamp_mediahost::pcm::{NamedPcmRings, PcmRings};
use rustamp_proto::host::PcmSpecWire;

const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44100,
    channels: 2,
};

#[test]
fn named_mapping_and_wake_transfer_then_drain_closed_ring() {
    let mut offer = NamedPcmRings.create(SPEC, 1).unwrap();
    let mut input = NamedPcmRings.open(&offer.message, SPEC).unwrap();
    assert_eq!(
        offer
            .producer
            .write_from_blocking(&[0.25, -0.5], Some(Duration::from_secs(1)))
            .unwrap(),
        1
    );
    offer.producer.close();
    let mut out = [0.0; 2];
    assert_eq!(
        input.consumer.read_into_blocking(&mut out, None).unwrap(),
        1
    );
    assert_eq!(out, [0.25, -0.5]);
    assert!(matches!(
        input.consumer.read_into_blocking(&mut out, None),
        Err(RingError::Closed)
    ));
    assert_eq!(input.consumer.frames_dropped(), 0);
}

#[test]
fn format_direction_and_generation_are_validated_before_ready() {
    let offer = NamedPcmRings.create(SPEC, 3).unwrap();
    let wrong = PcmSpecWire {
        channels: 1,
        ..SPEC
    };
    assert!(matches!(
        NamedPcmRings.open(&offer.message, wrong),
        Err(MediaHostError::Ring(RingError::FrameSizeMismatch { .. }))
    ));
    let mut stale = offer.message.clone();
    if let Message::RingOffer { generation, .. } = &mut stale {
        *generation = generation.wrapping_add(1);
    }
    assert!(matches!(
        NamedPcmRings.open(&stale, SPEC),
        Err(MediaHostError::Ring(RingError::Stale { .. }))
    ));
    let mut wrong_direction = offer.message;
    if let Message::RingOffer { direction, .. } = &mut wrong_direction {
        *direction = RingDirection::MainToHost;
    }
    assert!(matches!(
        NamedPcmRings.open(&wrong_direction, SPEC),
        Err(MediaHostError::InvalidMessage(_))
    ));
}

#[test]
fn invalid_format_capacity_and_size_are_refused_without_allocating() {
    assert!(NamedPcmRings.create(SPEC, 0).is_err());
    assert!(NamedPcmRings.create(SPEC, u32::MAX).is_err());
    assert!(
        NamedPcmRings
            .create(
                PcmSpecWire {
                    channels: 0,
                    ..SPEC
                },
                1
            )
            .is_err()
    );
    let mut offer = NamedPcmRings.create(SPEC, 3).unwrap();
    if let Message::RingOffer { size, .. } = &mut offer.message {
        *size += 1;
    }
    assert!(matches!(
        NamedPcmRings.open(&offer.message, SPEC),
        Err(MediaHostError::Ring(RingError::InvalidLayout))
    ));
}
