use anyhow::Result;
#[cfg(not(feature = "audio"))]
use anyhow::bail;

use crate::omega_public_channel_sarah::CommunityRoomAdmission;

pub const COMMUNITY_AUDIO_SAMPLE_RATE: u32 = 24_000;

#[derive(Clone, Debug)]
pub enum CommunityRoomMediaControl {
    SetMuted(bool),
    UpdateVerifiedParticipants(Vec<String>),
    Close,
}

#[derive(Clone, Debug)]
pub enum CommunityRoomMediaEvent {
    Connected,
    Audio(Vec<u8>),
    SarahSpeaking(bool),
    Reconnecting,
    Reconnected,
    RosterRefreshRequired,
    Ended,
    Error(String),
}

pub struct CommunityRoomMedia {
    pub controls: async_channel::Sender<CommunityRoomMediaControl>,
    pub events: async_channel::Receiver<CommunityRoomMediaEvent>,
    pub task: gpui::Task<Result<(), gpui_tokio::JoinError>>,
}

#[cfg(feature = "audio")]
mod enabled {
    use std::{
        collections::HashSet,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    use anyhow::{Context as _, Result, bail};
    use futures::{FutureExt as _, StreamExt as _, pin_mut, select_biased};
    use livekit::{
        Room, RoomEvent, RoomOptions,
        options::TrackPublishOptions,
        track::{LocalAudioTrack, LocalTrack, RemoteTrack, TrackKind, TrackSource},
        webrtc::{
            audio_frame::AudioFrame,
            audio_source::{AudioSourceOptions, RtcAudioSource, native::NativeAudioSource},
            audio_stream::native::NativeAudioStream,
        },
    };
    use rodio::{buffer::SamplesBuffer, nz};

    use audio::RodioExt as _;

    use super::{
        COMMUNITY_AUDIO_SAMPLE_RATE, CommunityRoomAdmission, CommunityRoomMedia,
        CommunityRoomMediaControl, CommunityRoomMediaEvent,
    };

    const MICROPHONE_CHUNK_SAMPLES: usize = 480;

    pub struct CommunityRoomPlayback {
        player: rodio::Player,
        _output: rodio::MixerDeviceSink,
    }

    impl CommunityRoomPlayback {
        fn open(echo_canceller: audio::EchoCanceller) -> Result<Self> {
            let (output, mixer) = audio::open_output_stream(None, echo_canceller)?;
            Ok(Self {
                player: rodio::Player::connect_new(&mixer),
                _output: output,
            })
        }

        pub fn play(&self, bytes: &[u8]) -> Result<()> {
            if !bytes.len().is_multiple_of(2) {
                bail!("community room audio had an invalid sample boundary");
            }
            let samples = bytes
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / i16::MAX as f32)
                .collect::<Vec<_>>();
            self.player
                .append(SamplesBuffer::new(nz!(1), nz!(24_000), samples));
            Ok(())
        }
    }

    struct MicrophoneCapture {
        receiver: async_channel::Receiver<Vec<u8>>,
        stop: Arc<AtomicBool>,
        _thread: thread::JoinHandle<()>,
    }

    impl MicrophoneCapture {
        fn start(mut echo_canceller: audio::EchoCanceller) -> Result<Self> {
            let microphone = audio::open_input_stream(None)?;
            let processing_failed = Arc::new(AtomicBool::new(false));
            let processing_failed_for_audio = processing_failed.clone();
            let mut microphone = microphone
                .constant_params(nz!(2), nz!(48_000))
                .process_buffer::<960, _>(move |buffer| {
                    let mut pcm16: [i16; 960] = std::array::from_fn(|index| {
                        (buffer[index].clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
                    });
                    if let Err(error) = echo_canceller.process_stream(&mut pcm16) {
                        log::error!("community microphone echo cancellation failed: {error:#}");
                        processing_failed_for_audio.store(true, Ordering::Release);
                        return;
                    }
                    for (sample, processed) in buffer.iter_mut().zip(pcm16) {
                        *sample = processed as f32 / i16::MAX as f32;
                    }
                })
                .possibly_disconnected_channels_to_mono()
                .constant_samplerate(nz!(24_000));
            let (sender, receiver) = async_channel::bounded(25);
            let stop = Arc::new(AtomicBool::new(false));
            let thread = thread::Builder::new()
                .name("CommunityRoomMicrophone".into())
                .spawn({
                    let stop = stop.clone();
                    move || {
                        let mut samples = Vec::with_capacity(MICROPHONE_CHUNK_SAMPLES);
                        while !stop.load(Ordering::Acquire)
                            && !processing_failed.load(Ordering::Acquire)
                        {
                            let Some(sample) = microphone.next() else {
                                break;
                            };
                            samples.push(sample);
                            if samples.len() < MICROPHONE_CHUNK_SAMPLES {
                                continue;
                            }
                            let mut bytes = Vec::with_capacity(samples.len() * 2);
                            for sample in samples.drain(..) {
                                let sample =
                                    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
                                bytes.extend_from_slice(&sample.to_le_bytes());
                            }
                            if sender.send_blocking(bytes).is_err() {
                                break;
                            }
                        }
                    }
                })
                .context("starting the community room microphone")?;
            Ok(Self {
                receiver,
                stop,
                _thread: thread,
            })
        }
    }

    impl Drop for MicrophoneCapture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
        }
    }

    struct Transport {
        livekit_url: String,
        room_ref: String,
        participant_ref: String,
        sarah_participant_ref: String,
        participant_grant: String,
        verified_participants: HashSet<String>,
    }

    pub fn start<C: gpui::AppContext>(
        admission: CommunityRoomAdmission,
        cx: &C,
    ) -> Result<(CommunityRoomMedia, CommunityRoomPlayback)> {
        let echo_canceller = audio::EchoCanceller::default();
        let microphone = MicrophoneCapture::start(echo_canceller.clone())
            .context("opening the community room microphone")?;
        let playback = CommunityRoomPlayback::open(echo_canceller)
            .context("opening the community room speaker")?;
        let transport = Transport {
            livekit_url: admission.livekit_url,
            room_ref: admission.room_ref,
            participant_ref: admission.participant_ref,
            sarah_participant_ref: admission.sarah_participant_ref,
            participant_grant: admission.participant_grant,
            verified_participants: admission
                .authority
                .verified_participants
                .into_iter()
                .map(|participant| participant.participant_ref)
                .collect(),
        };
        let (control_sender, control_receiver) = async_channel::bounded(16);
        let (event_sender, event_receiver) = async_channel::bounded(128);
        let task = gpui_tokio::Tokio::spawn(cx, async move {
            if let Err(error) = run_livekit_media(
                transport,
                microphone,
                control_receiver,
                event_sender.clone(),
            )
            .await
                && event_sender
                    .send(CommunityRoomMediaEvent::Error(format!("{error:#}")))
                    .await
                    .is_err()
            {
                log::debug!("community room media receiver closed while reporting failure");
            }
        });
        Ok((
            CommunityRoomMedia {
                controls: control_sender,
                events: event_receiver,
                task,
            },
            playback,
        ))
    }

    async fn run_livekit_media(
        mut transport: Transport,
        microphone: MicrophoneCapture,
        controls: async_channel::Receiver<CommunityRoomMediaControl>,
        events: async_channel::Sender<CommunityRoomMediaEvent>,
    ) -> Result<()> {
        let mut options = RoomOptions::default();
        options.auto_subscribe = false;
        let livekit_url = std::mem::take(&mut transport.livekit_url);
        let participant_grant = std::mem::take(&mut transport.participant_grant);
        let connection = Room::connect(&livekit_url, &participant_grant, options).fuse();
        let close_before_connect = controls.recv().fuse();
        pin_mut!(connection, close_before_connect);
        let (room, mut room_events) = select_biased! {
            control = close_before_connect => {
                match control {
                    Ok(CommunityRoomMediaControl::Close) | Err(_) => return Ok(()),
                    Ok(_) => bail!("community media received control before connecting"),
                }
            }
            connection = connection => connection.context("connecting the community LiveKit room")?,
        };
        let source = NativeAudioSource::new(
            AudioSourceOptions {
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
            },
            COMMUNITY_AUDIO_SAMPLE_RATE,
            1,
            100,
        );
        let mut audio_stream_tasks = tokio::task::JoinSet::new();
        let result = async {
            if room.name() != transport.room_ref
                || room.local_participant().identity().to_string() != transport.participant_ref
            {
                bail!("LiveKit joined outside the verified community admission");
            }
            subscribe_verified_sarah(&room, &transport, &events).await;

            let track = LocalAudioTrack::create_audio_track(
                "microphone",
                RtcAudioSource::Native(source.clone()),
            );
            room.local_participant()
                .publish_track(
                    LocalTrack::Audio(track),
                    TrackPublishOptions {
                        source: TrackSource::Microphone,
                        ..Default::default()
                    },
                )
                .await
                .context("publishing the community room microphone")?;
            send_event(&events, CommunityRoomMediaEvent::Connected).await;
            let mut muted = true;

            loop {
                let control = controls.recv().fuse();
                let audio = microphone.receiver.recv().fuse();
                let room_event = room_events.recv().fuse();
                pin_mut!(control, audio, room_event);
                select_biased! {
                    control = control => match control {
                        Ok(CommunityRoomMediaControl::SetMuted(value)) => muted = value,
                        Ok(CommunityRoomMediaControl::UpdateVerifiedParticipants(participants)) => {
                            transport.verified_participants = participants.into_iter().collect();
                            subscribe_verified_sarah(&room, &transport, &events).await;
                        }
                        Ok(CommunityRoomMediaControl::Close) | Err(_) => return Ok(()),
                    },
                    room_event = room_event => {
                        let Some(room_event) = room_event else {
                            send_event(&events, CommunityRoomMediaEvent::Ended).await;
                            return Ok(());
                        };
                        match room_event {
                            RoomEvent::ParticipantConnected(participant) => {
                                let identity = participant.identity().to_string();
                                if !verified_remote(&transport, &identity) {
                                    send_event(&events, CommunityRoomMediaEvent::RosterRefreshRequired).await;
                                }
                            }
                            RoomEvent::ParticipantDisconnected(participant) => {
                                if participant.identity().to_string() == transport.sarah_participant_ref {
                                    send_event(&events, CommunityRoomMediaEvent::Ended).await;
                                }
                            }
                            RoomEvent::TrackPublished { publication, participant } => {
                                let identity = participant.identity().to_string();
                                if should_subscribe(&transport, &identity, publication.kind()) {
                                    publication.set_subscribed(true);
                                } else if !verified_remote(&transport, &identity) {
                                    send_event(&events, CommunityRoomMediaEvent::RosterRefreshRequired).await;
                                }
                            }
                            RoomEvent::TrackSubscribed { track, participant, .. } => {
                                let identity = participant.identity().to_string();
                                if identity != transport.sarah_participant_ref {
                                    continue;
                                }
                                let RemoteTrack::Audio(track) = track else {
                                    continue;
                                };
                                let events = events.clone();
                                audio_stream_tasks.spawn(async move {
                                    let mut stream = NativeAudioStream::new(
                                        track.rtc_track(),
                                        COMMUNITY_AUDIO_SAMPLE_RATE as i32,
                                        1,
                                    );
                                    while let Some(frame) = stream.next().await {
                                        let mut bytes = Vec::with_capacity(frame.data.len() * 2);
                                        for sample in frame.data.iter() {
                                            bytes.extend_from_slice(&sample.to_le_bytes());
                                        }
                                        if events.send(CommunityRoomMediaEvent::Audio(bytes)).await.is_err() {
                                            break;
                                        }
                                    }
                                });
                            }
                            RoomEvent::ActiveSpeakersChanged { speakers } => {
                                let sarah_speaking = speakers.iter().any(|participant| {
                                    participant.identity().to_string()
                                        == transport.sarah_participant_ref
                                });
                                send_event(
                                    &events,
                                    CommunityRoomMediaEvent::SarahSpeaking(sarah_speaking),
                                )
                                .await;
                            }
                            RoomEvent::Reconnecting => {
                                send_event(&events, CommunityRoomMediaEvent::Reconnecting).await;
                            }
                            RoomEvent::Reconnected => {
                                send_event(&events, CommunityRoomMediaEvent::Reconnected).await;
                            }
                            RoomEvent::Disconnected { .. } => {
                                send_event(&events, CommunityRoomMediaEvent::Ended).await;
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                    audio = audio => {
                        let bytes = audio.context("community microphone capture stopped")?;
                        if muted {
                            continue;
                        }
                        if !bytes.len().is_multiple_of(2) {
                            bail!("community microphone emitted an invalid sample boundary");
                        }
                        let samples = bytes
                            .chunks_exact(2)
                            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
                            .collect::<Vec<_>>();
                        for samples in samples.chunks((COMMUNITY_AUDIO_SAMPLE_RATE / 100) as usize) {
                            if samples.len() != (COMMUNITY_AUDIO_SAMPLE_RATE / 100) as usize {
                                continue;
                            }
                            source.capture_frame(&AudioFrame {
                                data: samples.to_vec().into(),
                                sample_rate: COMMUNITY_AUDIO_SAMPLE_RATE,
                                num_channels: 1,
                                samples_per_channel: COMMUNITY_AUDIO_SAMPLE_RATE / 100,
                            }).await.context("sending a community LiveKit microphone frame")?;
                        }
                    }
                }
            }
        }
        .await;
        source.clear_buffer();
        match tokio::time::timeout(Duration::from_secs(5), room.close()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::debug!("community LiveKit room close failed: {error}"),
            Err(_) => log::debug!("community LiveKit room close timed out"),
        }
        audio_stream_tasks.shutdown().await;
        result
    }

    fn verified_remote(transport: &Transport, participant_ref: &str) -> bool {
        participant_ref == transport.sarah_participant_ref
            || transport.verified_participants.contains(participant_ref)
    }

    fn should_subscribe(transport: &Transport, participant_ref: &str, kind: TrackKind) -> bool {
        participant_ref == transport.sarah_participant_ref && kind == TrackKind::Audio
    }

    async fn subscribe_verified_sarah(
        room: &Room,
        transport: &Transport,
        events: &async_channel::Sender<CommunityRoomMediaEvent>,
    ) {
        for (identity, participant) in room.remote_participants() {
            let identity = identity.to_string();
            if !verified_remote(transport, &identity) {
                send_event(events, CommunityRoomMediaEvent::RosterRefreshRequired).await;
                continue;
            }
            for publication in participant.track_publications().into_values() {
                if should_subscribe(transport, &identity, publication.kind()) {
                    publication.set_subscribed(true);
                }
            }
        }
    }

    async fn send_event(
        events: &async_channel::Sender<CommunityRoomMediaEvent>,
        event: CommunityRoomMediaEvent,
    ) {
        if events.send(event).await.is_err() {
            log::debug!("community room media event receiver closed");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn transport() -> Transport {
            Transport {
                livekit_url: "wss://livekit.openagents.com".into(),
                room_ref: "room:verified".into(),
                participant_ref: "member-local".into(),
                sarah_participant_ref: "principal.sarah".into(),
                participant_grant: "grant".into(),
                verified_participants: HashSet::from([
                    "member-local".into(),
                    "member-remote".into(),
                ]),
            }
        }

        #[test]
        fn only_verified_sarah_audio_is_subscribed() {
            let transport = transport();
            assert!(verified_remote(&transport, "principal.sarah"));
            assert!(verified_remote(&transport, "member-remote"));
            assert!(!verified_remote(&transport, "forged-participant"));
            assert!(should_subscribe(
                &transport,
                "principal.sarah",
                TrackKind::Audio
            ));
            assert!(!should_subscribe(
                &transport,
                "member-remote",
                TrackKind::Audio
            ));
            assert!(!should_subscribe(
                &transport,
                "principal.sarah",
                TrackKind::Video
            ));
        }
    }
}

#[cfg(feature = "audio")]
pub use enabled::CommunityRoomPlayback;

#[cfg(feature = "audio")]
pub fn start_community_room_media<C: gpui::AppContext>(
    admission: CommunityRoomAdmission,
    cx: &C,
) -> Result<(CommunityRoomMedia, CommunityRoomPlayback)> {
    enabled::start(admission, cx)
}

#[cfg(not(feature = "audio"))]
pub struct CommunityRoomPlayback;

#[cfg(not(feature = "audio"))]
impl CommunityRoomPlayback {
    pub fn play(&self, _bytes: &[u8]) -> Result<()> {
        bail!("community room audio is not available in this build")
    }
}

#[cfg(not(feature = "audio"))]
pub fn start_community_room_media<C: gpui::AppContext>(
    _admission: CommunityRoomAdmission,
    _cx: &C,
) -> Result<(CommunityRoomMedia, CommunityRoomPlayback)> {
    bail!("community room audio is not available in this build")
}
