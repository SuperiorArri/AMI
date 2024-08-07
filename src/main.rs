use audio::output::{BufferTx, DefaultOutputDeviceParams};
use clap::Parser;
use control::{
    controller::{self, Controller},
    node::drum_machine,
};
use midi::MidiReader;
use render::{
    node::{fluidlite_synth, oxi_synth, rusty_synth, sfizz_synth},
    renderer::{self, Renderer},
};
use ringbuf::traits::Producer;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicUsize, Arc},
    time::Duration,
};
use tokio::sync::Mutex;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use webserver::{Clients, ServerMessageKind};

pub mod audio;
pub mod control;
pub mod json;
pub mod midi;
pub mod path;
pub mod render;
pub mod rhythm;
pub mod synth;
mod webserver;

const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(about = "Simple software for adding two integers numbers.")]

//TODO: implement node container which will provice basic functionality like midi filters and
//  velocity mapping OR make macros to implement this functionality automatically
pub struct Args {
    // #[clap(index=1)]
    // a: i32,

    // #[clap(index=2)]
    // b: Option<i32>,
    #[arg(short, long, help = "Path to samples directory")]
    samples: PathBuf,

    #[arg(short, long, help = "Path to beats directory")]
    beats: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!(
        "Starting AMI (Arri's Midi Instrument) > version: {}",
        VERSION.unwrap_or("?")
    );

    info!("| Samples directory: {:?}", args.samples);
    info!("| Beats directory: {:?}", args.beats);

    let (midi_tx, midi_rx) = midi::create_channel(2048);
    let (rnd_req_tx, rnd_req_rx) = renderer::create_request_channel(32);
    let (ctr_req_tx, ctr_req_rx) = controller::create_request_channel(32);
    let (ctr_tx, ctr_rx) = control::create_control_channel(32);

    let mut virtual_paths = crate::path::VirtualPaths::default();
    virtual_paths.insert("samples:".into(), args.samples);
    virtual_paths.insert("beats:".into(), args.beats);

    info!("| Available MIDI ports:");
    for port in midi::MidiReader::get_available_ports() {
        info!("| - {port}");
    }

    let clients = Clients::new(256);
    let mut midi_reader = midi::MidiReader::with_slots(midi_tx.clone(), 16);

    if midi_reader
        .connect_input(0, "VMPK Output:out 130:0")
        .is_ok()
    {
        tracing::debug!("MIDI port connected: VMPK Output:out 130:0");
    }
    if midi_reader
        .connect_input(0, "Hammer 88 Pro:Hammer 88 Pro USB MIDI 20:0")
        .is_ok()
    {
        tracing::debug!("MIDI port connected: Hammer 88 Pro:Hammer 88 Pro USB MIDI 20:0");
    }

    let midi_reader = Arc::new(Mutex::new(midi_reader));

    tokio::spawn(run_midi_logger(midi_rx, clients.clone()));
    tokio::spawn(run_midi_port_watchdog(clients.clone()));

    let mut cache = webserver::Cache::default();

    #[cfg(not(target_os = "windows"))]
    let sample_rate = 44100;

    #[cfg(target_os = "windows")]
    let sample_rate = 48000;

    let buffer_size = 2048;

    let audio_output = audio::output::connect_to_default_output_device(DefaultOutputDeviceParams {
        sample_rate,
        buffer_size,
        num_channels: 2,
    })
    .expect("Failed to connect to output device");

    let mut renderer = Renderer::new(
        midi_tx.subscribe(),
        rnd_req_rx,
        ctr_rx,
        virtual_paths.clone(),
        clients.clone(),
        cache.clone(),
    );
    renderer.register_node_kind("RustySynth", || Box::<rusty_synth::Node>::default());
    renderer.register_node_kind("OxiSynth", || Box::<oxi_synth::Node>::default());
    renderer.register_node_kind("FluidliteSynth", || Box::<fluidlite_synth::Node>::default());
    renderer.register_node_kind("SfizzSynth", || Box::<sfizz_synth::Node>::default());
    renderer.set_sample_rate(audio_output.sample_rate);

    let req_num_samples = audio_output.required_num_samples;
    let lbuf_tx = audio_output.lbuf_tx;
    let rbuf_tx = audio_output.rbuf_tx;
    tokio::spawn(run_renderer(renderer, req_num_samples, (lbuf_tx, rbuf_tx)));

    let mut controller = Controller::new(
        midi_tx.subscribe(),
        ctr_req_rx,
        ctr_tx,
        virtual_paths.clone(),
        clients.clone(),
        cache.clone(),
    );
    controller.register_node_kind("DrumMachine", || Box::<drum_machine::Node>::default());
    cache.set_controller(controller.serialize().await).await;

    tokio::spawn(run_controller(controller));

    let shared_state = webserver::SharedState {
        clients: Clients::clone(&clients),
        midi_reader: Arc::clone(&midi_reader),
        cache: cache.clone(),
    };

    webserver::run(3000, shared_state, move |addr, req| {
        let midi_reader = Arc::clone(&midi_reader);
        let mut clients = Clients::clone(&clients);
        let rnd_req_tx = rnd_req_tx.clone();
        let ctr_req_tx = ctr_req_tx.clone();
        let vp = virtual_paths.clone();
        async move {
            use webserver::ClientMessageKind;
            use webserver::ServerMessageKind;
            match req {
                ClientMessageKind::Ping => ServerMessageKind::Pong,
                ClientMessageKind::Report(report) => {
                    info!("Report from [{addr}]: {report}");
                    ServerMessageKind::Ack
                }
                ClientMessageKind::ConnectMidiInput(slot, name) => {
                    let mut midi_reader = midi_reader.lock().await;
                    if let Ok(()) = midi_reader.connect_input(slot, &name) {
                        clients.broadcast(ServerMessageKind::ConnectedMidiInputs(
                            midi_reader.connected_input_names(),
                        ));
                        ServerMessageKind::Ack
                    } else {
                        ServerMessageKind::Nak
                    }
                }
                ClientMessageKind::DisconnectMidiInput(slot) => {
                    let mut midi_reader = midi_reader.lock().await;
                    if let Ok(()) = midi_reader.disconnect_input(slot) {
                        clients.broadcast(ServerMessageKind::ConnectedMidiInputs(
                            midi_reader.connected_input_names(),
                        ));
                        ServerMessageKind::Ack
                    } else {
                        ServerMessageKind::Nak
                    }
                }
                ClientMessageKind::RendererRequest(req) => {
                    let res = send_renderer_request(&rnd_req_tx, req).await;
                    if let Some(res) = res {
                        ServerMessageKind::RendererResponse(res)
                    } else {
                        ServerMessageKind::Nak
                    }
                }
                ClientMessageKind::ControllerRequest(req) => {
                    let res = send_controller_request(&ctr_req_tx, req).await;
                    if let Some(res) = res {
                        ServerMessageKind::ControllerResponse(res)
                    } else {
                        ServerMessageKind::Nak
                    }
                }
                ClientMessageKind::ReadDir(path) => {
                    if let Some(path) = vp.translate(&path) {
                        if let Ok(dir) = std::fs::read_dir(&path) {
                            //TODO: use tokio instead
                            let entries = dir
                                .into_iter()
                                .flatten()
                                .map(|x| {
                                    (
                                        x.path().is_dir(),
                                        crate::path::remove_prefix(x.path().as_path(), &path),
                                    )
                                })
                                .collect();
                            return ServerMessageKind::DirInfo(Some(entries));
                        }
                    }
                    ServerMessageKind::DirInfo(None)
                }
                ClientMessageKind::MakeDir(path) => {
                    if let Some(path) = vp.translate(&path) {
                        if tokio::fs::create_dir_all(&path).await.is_ok() {
                            return ServerMessageKind::Ack;
                        }
                    }
                    ServerMessageKind::Nak
                }
                ClientMessageKind::DeleteFile(path) => {
                    if let Some(path) = vp.translate(&path) {
                        if path.is_dir() {
                            if tokio::fs::remove_dir_all(path).await.is_ok() {
                                return ServerMessageKind::Ack;
                            }
                        } else if path.is_file() && tokio::fs::remove_file(path).await.is_ok() {
                            return ServerMessageKind::Ack;
                        }
                    }
                    ServerMessageKind::Nak
                }
                ClientMessageKind::RenameFile(path, new_path) => {
                    if let (Some(path), Some(new_path)) =
                        (vp.translate(&path), vp.translate(&new_path))
                    {
                        if tokio::fs::rename(&path, &new_path).await.is_ok() {
                            return ServerMessageKind::Ack;
                        }
                    }
                    ServerMessageKind::Nak
                }
                ClientMessageKind::CopyFile(path, new_path) => ServerMessageKind::Nak,
            }
        }
    })
    .await;

    Ok(())
}

async fn run_midi_logger(mut midi_rx: midi::Receiver, mut clients: Clients) {
    while let Ok(message) = midi_rx.recv().await {
        // tracing::trace!("MSG");
        clients.broadcast(ServerMessageKind::MidiEvent(message));
    }
}

async fn run_midi_port_watchdog(mut clients: Clients) {
    loop {
        clients.broadcast(ServerMessageKind::AvailableMidiInputs(
            MidiReader::get_available_ports(),
        ));
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

async fn run_renderer(
    mut renderer: Renderer,
    req_num_samples: Arc<AtomicUsize>,
    (mut lbuf_tx, mut rbuf_tx): (BufferTx, BufferTx),
) {
    let mut lbuf = vec![];
    let mut rbuf = vec![];
    let mut counter = 0;

    loop {
        counter += 1;
        if counter >= 10 {
            counter = 0;
            renderer.update().await;
        }

        let curr_buf_size = req_num_samples.load(std::sync::atomic::Ordering::Relaxed);

        if curr_buf_size > 0 {
            if lbuf.len() < curr_buf_size {
                lbuf.resize(curr_buf_size, 0.0);
                rbuf.resize(curr_buf_size, 0.0);
            }

            let lbuf_slice = &mut lbuf[..curr_buf_size];
            let rbuf_slice = &mut rbuf[..curr_buf_size];

            renderer.render(lbuf_slice, rbuf_slice);

            lbuf_tx.push_slice(lbuf_slice);
            rbuf_tx.push_slice(rbuf_slice);

            req_num_samples.store(0, std::sync::atomic::Ordering::Relaxed);
        }

        tokio::time::sleep(Duration::from_micros(10)).await;
    }
}

async fn send_renderer_request(
    req_tx: &renderer::Requester,
    req: renderer::RequestKind,
) -> Option<renderer::ResponseKind> {
    let (res_tx, res_rx) = renderer::create_response_channel();

    if let Ok(()) = req_tx.send((req, res_tx)).await {
        if let Ok(response_kind) = res_rx.await {
            Some(response_kind)
        } else {
            None
        }
    } else {
        None
    }
}

async fn run_controller(mut controller: Controller) {
    loop {
        controller.update().await;
        tokio::time::sleep(Duration::from_secs_f32(controller.period().min(0.01))).await;
    }
}

async fn send_controller_request(
    req_tx: &controller::Requester,
    req: controller::RequestKind,
) -> Option<controller::ResponseKind> {
    let (res_tx, res_rx) = controller::create_response_channel();

    if let Ok(()) = req_tx.send((req, res_tx)).await {
        if let Ok(response_kind) = res_rx.await {
            Some(response_kind)
        } else {
            None
        }
    } else {
        None
    }
}

async fn play_midi_file(path: &Path, midi_tx: midi::Sender) {
    let data = std::fs::read(path).unwrap();
    let smf = midly::Smf::parse(&data).unwrap();
    let timing = smf.header.timing;

    let mut max_num_events = 0;
    for track in &smf.tracks {
        max_num_events += track.len();
    }
    let mut events = Vec::with_capacity(max_num_events);

    enum Event {
        Tempo(f32),
        Midi(midi::Message),
    }

    for track in smf.tracks.iter() {
        let mut time: u128 = 0;
        for e in track {
            time += e.delta.as_int() as u128;
            if let midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) = e.kind {
                let tempo_bpm = 60000000.0 / t.as_int() as f32;
                events.push((time, Event::Tempo(tempo_bpm)));
            } else if let Some(msg) = midly_event_to_midi_message(&e.kind) {
                events.push((time, Event::Midi(msg)));
            }
        }
    }

    events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut time: u128 = 0;

    for event in &mut events {
        let new_time = event.0;
        event.0 -= time;
        time = new_time;
    }

    let mut delta_coef = timing_to_sec(timing, 90.0);
    for (dt, event) in events {
        match event {
            Event::Tempo(bpm) => delta_coef = timing_to_sec(timing, bpm),
            Event::Midi(msg) => {
                if dt > 0 {
                    tokio::time::sleep(Duration::from_secs_f32(dt as f32 * delta_coef)).await;
                }
                _ = midi_tx.send(msg);
            }
        }
    }
}

fn midly_event_to_midi_message(kind: &midly::TrackEventKind) -> Option<midi::Message> {
    if let midly::TrackEventKind::Midi { channel, message } = kind {
        let kind = match message {
            midly::MidiMessage::NoteOff { key, vel } => Some(midi::MessageKind::NoteOff {
                note: key.as_int(),
                velocity: vel.as_int(),
            }),
            midly::MidiMessage::NoteOn { key, vel } => Some(midi::MessageKind::NoteOn {
                note: key.as_int(),
                velocity: vel.as_int(),
            }),
            midly::MidiMessage::Aftertouch { key, vel } => {
                Some(midi::MessageKind::PolyphonicAftertouch {
                    note: key.as_int(),
                    pressure: vel.as_int(),
                })
            }
            midly::MidiMessage::Controller { controller, value } => {
                let kind = midi::ControlChangeKind::from_number(controller.as_int())?;
                Some(midi::MessageKind::ControlChange {
                    kind,
                    value: value.as_int(),
                })
            }
            midly::MidiMessage::ProgramChange { program } => {
                Some(midi::MessageKind::ProgramChange {
                    program: program.as_int(),
                })
            }
            midly::MidiMessage::ChannelAftertouch { vel } => {
                Some(midi::MessageKind::ChannelAftertouch {
                    pressure: vel.as_int(),
                })
            }
            midly::MidiMessage::PitchBend { bend } => Some(midi::MessageKind::PitchWheel {
                value: bend.as_int() as u16,
            }),
        };
        Some(midi::Message {
            kind: kind?,
            channel: channel.as_int(),
        })
    } else {
        None
    }
}

fn timing_to_sec(timing: midly::Timing, tempo_bpm: f32) -> f32 {
    match timing {
        midly::Timing::Metrical(tpb) => 60.0 / (tempo_bpm * tpb.as_int() as f32),
        midly::Timing::Timecode(fps, subframe) => 1.0 / fps.as_f32() / (subframe as f32),
    }
}
