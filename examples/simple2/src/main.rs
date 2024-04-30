use futures::{select, FutureExt};
use futures_timer::Delay;
use log::info;
use matchbox_socket::{PeerState, WebRtcSocket};
use schnorrkel::olaf::simplpedpop::{
    round1::{self, PublicMessage},
    round2::{self, Messages},
    Parameters,
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Write, path::Path, time::Duration};

#[cfg(target_arch = "wasm32")]
fn main() {
    // Setup logging
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).unwrap();

    wasm_bindgen_futures::spawn_local(async_main());
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
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

async fn async_main() {
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
                            .unwrap();

                    let dir_path = "round1";

                    // Serialize and save the public message
                    let public_message_json =
                        serde_json::to_string_pretty(&public_message).unwrap();

                    let mut public_message_file =
                        File::create(Path::new(&dir_path).join("public_message.json")).unwrap();

                    public_message_file
                        .write_all(public_message_json.as_bytes())
                        .unwrap();

                    // Serialize and save the private and public data together
                    let combined_data = CombinedData {
                        private_data,
                        public_data,
                    };

                    let combined_data_json = serde_json::to_string_pretty(&combined_data).unwrap();

                    let mut combined_data_file =
                        File::create(Path::new(&dir_path).join("combined_data.json")).unwrap();

                    combined_data_file
                        .write_all(combined_data_json.as_bytes())
                        .unwrap();

                    println!("Data saved to directory {}", dir_path);

                    let packet =
                        bincode::serialize(&ProtocolMessage::Round1Message(public_message))
                            .unwrap()
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

            let message: ProtocolMessage = bincode::deserialize(&packet).unwrap();
            match message {
                ProtocolMessage::Round1Message(data) => {
                    println!("Received Round 1 message with data: {:?}", data);
                    // Handle Round 1 message
                }
                ProtocolMessage::Round2Message(data) => {
                    println!("Received Round 2 message with data: {:?}", data);
                    // Handle Round 2 message
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
                break;
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
