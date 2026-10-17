//! Windows directed-ring validation: the DSP in-ring flows MainToHost while
//! the decoder path stays HostToMain. No windows or audio.

#![cfg(windows)]

use rustamp_ipc::Message;
use rustamp_ipc::shm::{RingDirection, RingError};
use rustamp_mediahost::MediaHostError;
use rustamp_mediahost::pcm::{NamedPcmRings, PcmRings};
use rustamp_proto::host::PcmSpecWire;

const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

#[test]
fn directed_rings_carry_both_directions_losslessly() {
    let mut offer = NamedPcmRings
        .create_directed(SPEC, 2, RingDirection::MainToHost)
        .unwrap();
    if let Message::RingOffer { direction, .. } = &offer.message {
        assert_eq!(*direction, RingDirection::MainToHost);
    } else {
        panic!("expected an offer");
    }
    // The wrong direction is refused before any mapping opens.
    assert!(matches!(
        NamedPcmRings.open_directed(&offer.message, SPEC, RingDirection::HostToMain),
        Err(MediaHostError::InvalidMessage(_))
    ));
    let mut input = NamedPcmRings
        .open_directed(&offer.message, SPEC, RingDirection::MainToHost)
        .unwrap();
    assert_eq!(
        offer
            .producer
            .write_from_blocking(
                &[0.25, -0.5, 0.75, 1.0],
                Some(std::time::Duration::from_secs(1))
            )
            .unwrap(),
        2
    );
    offer.producer.close();
    let mut out = [0.0; 4];
    assert_eq!(
        input.consumer.read_into_blocking(&mut out, None).unwrap(),
        2
    );
    assert_eq!(out, [0.25, -0.5, 0.75, 1.0]);
    assert!(matches!(
        input.consumer.read_into_blocking(&mut out, None),
        Err(RingError::Closed)
    ));
    assert_eq!(input.consumer.frames_dropped(), 0);
}

#[test]
fn decoder_defaults_still_offer_host_to_main() {
    // The direction generalisation must not move the decoder path: the plain
    // entry points keep offering HostToMain rings.
    let offer = NamedPcmRings.create(SPEC, 1).unwrap();
    if let Message::RingOffer { direction, .. } = &offer.message {
        assert_eq!(*direction, RingDirection::HostToMain);
    } else {
        panic!("expected an offer");
    }
    assert!(NamedPcmRings.open(&offer.message, SPEC).is_ok());
}

#[test]
fn invalid_format_and_size_are_refused_before_allocating() {
    assert!(
        NamedPcmRings
            .create_directed(SPEC, 0, RingDirection::MainToHost)
            .is_err()
    );
    assert!(
        NamedPcmRings
            .create_directed(
                PcmSpecWire {
                    channels: 0,
                    ..SPEC
                },
                1,
                RingDirection::MainToHost
            )
            .is_err()
    );
}
