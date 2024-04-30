use futures::{select, FutureExt};
use futures_timer::Delay;
use log::info;
use matchbox_socket::{PeerState, WebRtcSocket};
use merlin::Transcript;
use schnorrkel::{
    olaf::{
        errors::DKGError,
        identifier::Identifier,
        keys::GroupPublicKeyShare,
        simplpedpop::{
            round1::{self, PublicMessage},
            round2::{self, Messages},
            round3::{self, PrivateData},
            Parameters,
        },
    },
    PublicKey,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::Path,
    time::Duration,
};

const ROUND1_DIR_PATH: &'static str = "round1";
const ROUND2_DIR_PATH: &'static str = "round2";
const ROUND3_DIR_PATH: &'static str = "round3";

pub type MyResult<T> = Result<T, MyError>;

#[cfg(target_arch = "wasm32")]
fn main() {
    // Setup logging
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).unwrap();

    wasm_bindgen_futures::spawn_local(async_main());
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> MyResult<()> {
    // Setup logging
    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "simple_example=info,matchbox_socket=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    async_main().await
}

async fn async_main() -> MyResult<()> {
    // Define the role of this client instance: "pinger" or "ponger"
    let role = "pinger"; // This could also be dynamically set, e.g., from an environment variable

    info!("Connecting to matchbox as {}", role);
    let (mut socket, loop_fut) = WebRtcSocket::new_reliable("ws://localhost:3536/");

    let loop_fut = loop_fut.fuse();
    futures::pin_mut!(loop_fut);

    let timeout = Delay::new(Duration::from_secs(1)); // Adjusted for ping/pong frequency
    futures::pin_mut!(timeout);

    loop {
        // Handle any new peers
        for (peer, state) in socket.update_peers() {
            match state {
                PeerState::Connected => {
                    info!("Peer joined: {}", peer);

                    let parameters = Parameters::new(2, 2);

                    let (private_data, public_message, public_data) =
                        schnorrkel::olaf::simplpedpop::round1::run(parameters, rand_core::OsRng)
                            .map_err(MyError::Dkg)?;

                    // Serialize and save the public message
                    let public_message_json =
                        serde_json::to_string_pretty(&public_message).map_err(MyError::Json)?;

                    let mut public_message_file =
                        File::create(Path::new(&ROUND1_DIR_PATH).join("public_message.json"))
                            .map_err(MyError::Io)?;

                    public_message_file
                        .write_all(public_message_json.as_bytes())
                        .map_err(MyError::Io)?;

                    // Serialize and save the private and public data together
                    let combined_data = CombinedData {
                        private_data,
                        public_data,
                    };

                    let combined_data_json =
                        serde_json::to_string_pretty(&combined_data).map_err(MyError::Json)?;

                    let mut combined_data_file =
                        File::create(Path::new(&ROUND1_DIR_PATH).join("combined_data.json"))
                            .map_err(MyError::Io)?;

                    combined_data_file
                        .write_all(combined_data_json.as_bytes())
                        .map_err(MyError::Io)?;

                    println!("Data saved to directory {}", ROUND1_DIR_PATH);

                    let packet =
                        bincode::serialize(&ProtocolMessage::Round1Message(public_message))
                            .map_err(MyError::Bincode)?
                            .into_boxed_slice();

                    socket.send(packet.clone(), peer);
                }
                PeerState::Disconnected => {
                    info!("Peer left: {}", peer);
                }
            }
        }

        // Accept any messages incoming
        for (peer, packet) in socket.receive() {
            //let message = String::from_utf8_lossy(&packet);
            //info!("Message from {}: {}", peer, message);

            let message: ProtocolMessage =
                bincode::deserialize(&packet).map_err(MyError::Bincode)?;

            match message {
                ProtocolMessage::Round1Message(data) => {
                    println!("Received Round 1 message");
                    // Handle Round 1 message

                    // Deserialize and read the combined private and public data
                    let combined_data_json =
                        fs::read_to_string(Path::new(&ROUND1_DIR_PATH).join("combined_data.json"))
                            .map_err(MyError::Io)?;

                    let combined_data: CombinedData =
                        serde_json::from_str(&combined_data_json).map_err(MyError::Json)?;

                    let message: round1::PublicMessage = data;

                    let mut messages = BTreeSet::new();
                    messages.insert(message);

                    let (public_data, messages) = round2::run(
                        combined_data.private_data,
                        &combined_data.public_data,
                        messages,
                        Transcript::new(b"label"),
                    )
                    .map_err(MyError::Dkg)?;

                    // Serialize and save the public data to output directory
                    let public_data_json =
                        serde_json::to_string_pretty(&public_data).map_err(MyError::Json)?;

                    let mut public_data_file =
                        File::create(Path::new(&ROUND2_DIR_PATH).join("public_data.json"))
                            .map_err(MyError::Io)?;

                    public_data_file
                        .write_all(public_data_json.as_bytes())
                        .map_err(MyError::Io)?;

                    // Serialize and save the messages to output directory
                    let messages_json =
                        serde_json::to_string_pretty(&messages).map_err(MyError::Json)?;

                    let mut messages_file =
                        File::create(Path::new(&ROUND2_DIR_PATH).join("messages.json"))
                            .map_err(MyError::Io)?;

                    messages_file
                        .write_all(messages_json.as_bytes())
                        .map_err(MyError::Io)?;

                    println!(
                        "Public data and messages saved to directory {}",
                        ROUND2_DIR_PATH
                    );

                    let packet = bincode::serialize(&ProtocolMessage::Round2Message(messages))
                        .map_err(MyError::Bincode)?
                        .into_boxed_slice();

                    socket.send(packet.clone(), peer);
                }
                ProtocolMessage::Round2Message(data) => {
                    println!("Received Round 2 message");
                    // Handle Round 2 message

                    // Deserialize and read the combined private and public data
                    let combined_data_json =
                        fs::read_to_string(Path::new(&ROUND1_DIR_PATH).join("combined_data.json"))
                            .map_err(MyError::Io)?;

                    let combined_data: CombinedData =
                        serde_json::from_str(&combined_data_json).map_err(MyError::Json)?;

                    let round2_public_data_json =
                        fs::read_to_string(Path::new(&ROUND2_DIR_PATH).join("public_data.json"))
                            .map_err(MyError::Io)?;

                    let round2_public_data: round2::PublicData =
                        serde_json::from_str(&round2_public_data_json).map_err(MyError::Json)?;

                    let identifier = round2_public_data
                        .identifiers()
                        .others_identifiers()
                        .first()
                        .unwrap();

                    let mut round2_public_messages: BTreeMap<Identifier, round2::PublicMessage> =
                        BTreeMap::new();

                    round2_public_messages.insert(*identifier, data.public_message().clone());

                    let mut round2_private_messages: BTreeMap<Identifier, round2::PrivateMessage> =
                        BTreeMap::new();

                    round2_private_messages.insert(
                        *identifier,
                        data.private_messages().first_key_value().unwrap().1.clone(),
                    );

                    // Run Round 3
                    let result = round3::run(
                        &round2_public_messages,
                        &round2_public_data,
                        &combined_data.public_data,
                        combined_data.private_data,
                        &round2_private_messages,
                    )
                    .map_err(MyError::Dkg)?;

                    let container = Container {
                        group_public_key: result.0,
                        group_public_key_shares: result.1,
                        private_data: result.2,
                    };

                    // Serialize and save the result of Round 3
                    let output_json =
                        serde_json::to_string_pretty(&container).map_err(MyError::Json)?;

                    let mut output_file =
                        File::create(Path::new(&ROUND3_DIR_PATH).join("round3_result.json"))
                            .map_err(MyError::Io)?;

                    output_file
                        .write_all(output_json.as_bytes())
                        .map_err(MyError::Io)?;

                    println!("Round 3 result saved to {}", ROUND3_DIR_PATH);
                }
            }
        }

        select! {
            // Restart this loop every 100ms
            _ = (&mut timeout).fuse() => {
                timeout.reset(Duration::from_millis(100));
            }

            // Or break if the message loop ends (disconnected, closed, etc.)
            _ = &mut loop_fut => {
                //break;
                return Ok(());
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CombinedData {
    private_data: round1::PrivateData,
    public_data: round1::PublicData,
}

// Define an enum to represent different types of messages for each round
#[derive(Serialize, Deserialize, Debug)]
enum ProtocolMessage {
    Round1Message(PublicMessage),
    Round2Message(Messages),
}

#[derive(Debug)]
enum MyError {
    Io(std::io::Error),
    Dkg(DKGError),
    Json(serde_json::Error),
    Bincode(bincode::Error),
}

impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            MyError::Io(ref err) => write!(f, "IO error: {}", err),
            MyError::Dkg(ref err) => write!(f, "DKG error: {}", err),
            MyError::Json(ref err) => write!(f, "Json error: {}", err),
            MyError::Bincode(ref err) => write!(f, "Bincode error: {}", err),
        }
    }
}

impl std::error::Error for MyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            MyError::Io(ref err) => Some(err),
            MyError::Dkg(ref err) => Some(err),
            MyError::Json(ref err) => Some(err),
            MyError::Bincode(ref err) => Some(err),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Container {
    group_public_key: PublicKey,
    group_public_key_shares: BTreeMap<Identifier, GroupPublicKeyShare>,
    private_data: PrivateData,
}
