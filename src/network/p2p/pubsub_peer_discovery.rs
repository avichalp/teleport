use std::{sync::Arc, time::Duration};

use libp2p::futures::{Future, FutureExt};
use libp2p::gossipsub::Event as GossipsubEvent;
use libp2p::swarm::SwarmEvent;
use libp2p::{gossipsub::IdentTopic, Swarm};
use std::pin::Pin;
use tokio::sync::Mutex;
use tokio::time;

use crate::core::errors::HubError;

use super::gossip_behaviour::{GossipBehaviour, GossipBehaviourEvent};
use super::handle_swarm_event::SwarmEventHandler;

pub struct PubSubPeerDiscovery {
    interval: Duration,
    listen_only: bool,
    is_started: bool,
    topic: IdentTopic,
    swarm: Arc<Mutex<Swarm<GossipBehaviour>>>,
    stop_signal: Arc<Mutex<bool>>,
}

impl PubSubPeerDiscovery {
    pub fn new(
        interval: Duration,
        listen_only: bool,
        swarm: Arc<Mutex<Swarm<GossipBehaviour>>>,
        topic: IdentTopic,
    ) -> Self {
        Self {
            interval,
            listen_only,
            is_started: false,
            topic,
            swarm,
            stop_signal: Arc::new(Mutex::new(false)),
        }
    }

    pub fn is_started(&self) -> bool {
        self.is_started
    }

    pub async fn start(&mut self) -> Result<(), HubError> {
        if self.is_started {
            return Ok(());
        }

        self.swarm
            .lock()
            .await
            .behaviour_mut()
            .gossipsub
            .subscribe(&self.topic)
            .unwrap();

        self.is_started = true;

        if self.listen_only {
            return Ok(());
        }

        broadcast(self.swarm.clone(), &self.topic).await;

        let stop_signal = self.stop_signal.clone();
        let swarm = self.swarm.clone();
        let topic = self.topic.clone();
        let interval = self.interval;

        // Periodically call broadcast again
        tokio::spawn(async move {
            let mut interval = time::interval(interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if *stop_signal.lock().await {
                            break;
                        }

                        broadcast(swarm.clone(), &topic).await;
                    }

                    _ = tokio::signal::ctrl_c() => {
                        *stop_signal.lock().await = true;
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), HubError> {
        if !self.is_started {
            return Ok(());
        }

        // Unsubscribe from the topics
        self.swarm
            .lock()
            .await
            .behaviour_mut()
            .gossipsub
            .unsubscribe(&self.topic)
            .unwrap();

        self.is_started = false;

        Ok(())
    }
}

impl SwarmEventHandler for PubSubPeerDiscovery {
    fn handle<'a>(
        &'a self,
        event: &'a SwarmEvent<GossipBehaviourEvent, std::io::Error>,
    ) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        async move {
            if !self.is_started {
                return;
            }

            if let SwarmEvent::Behaviour(event) = event {
                match event {
                    GossipBehaviourEvent::Gossipsub(event) => {
                        if let GossipsubEvent::Message {
                            propagation_source,
                            message_id: _,
                            message,
                        } = event
                        {
                            if self.topic.to_string() != message.topic.to_string() {
                                return;
                            }

                            let locked_swarm = self.swarm.lock().await;
                            let local_peer_id = locked_swarm.local_peer_id();

                            if local_peer_id == propagation_source {
                                return;
                            }

                            println!(
                                "Received message from {:?}: {:?}",
                                propagation_source, message
                            );

                            todo!("dial peer")
                        }
                    }
                    _ => {}
                }
            }
        }
        .boxed()
    }
}

pub async fn broadcast(swarm: Arc<Mutex<Swarm<GossipBehaviour>>>, topic: &IdentTopic) {
    // TODO: This is likely wrong - js-libp2p encodes using protobuf over
    // public key and multiaddresses
    let encoded_peer_id = swarm.lock().await.local_peer_id().to_bytes();

    let _ = swarm
        .lock()
        .await
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), encoded_peer_id);
}
