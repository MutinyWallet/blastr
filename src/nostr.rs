use crate::error::Error;
use futures::pin_mut;
use nostr::prelude::*;
use std::{time::Duration, vec};
use worker::{console_log, Cache, Delay, Fetch, Queue, Response, WebSocket};

pub(crate) const NOSTR_QUEUE: &str = "nostr-events-pub";
pub(crate) const NOSTR_QUEUE_2: &str = "nostr-events-pub-2";
pub(crate) const NOSTR_QUEUE_3: &str = "nostr-events-pub-3";
pub(crate) const NOSTR_QUEUE_4: &str = "nostr-events-pub-4";
pub(crate) const NOSTR_QUEUE_5: &str = "nostr-events-pub-5";
pub(crate) const NOSTR_QUEUE_6: &str = "nostr-events-pub-6";
const RELAY_LIST_URL: &str = "https://api.nostr.watch/v1/online";
const RELAYS: [&str; 8] = [
    "wss://nostr.zebedee.cloud",
    "wss://relay.snort.social",
    "wss://eden.nostr.land",
    "wss://nos.lol",
    "wss://brb.io",
    "wss://nostr.fmt.wiz.biz",
    "wss://relay.damus.io",
    "wss://nostr.wine",
];

pub async fn try_queue_event(event: Event, nostr_queues: Vec<Queue>) {
    for nostr_queue in nostr_queues.iter() {
        match queue_nostr_event_with_queue(nostr_queue, event.clone()).await {
            Ok(_) => {}
            Err(Error::WorkerError(e)) => {
                console_log!("worker error: {e}");
            }
        }
    }
}

pub async fn queue_nostr_event_with_queue(nostr_queue: &Queue, event: Event) -> Result<(), Error> {
    nostr_queue.send(&event).await?;
    Ok(())
}

async fn send_event_to_relay(messages: Vec<ClientMessage>, relay: &str) -> Result<(), Error> {
    // skip self
    if relay == "wss://nostr.mutinywallet.com" {
        return Ok(());
    }
    match WebSocket::connect(relay.parse().unwrap()).await {
        Ok(ws) => {
            // It's important that we call this before we send our first message, otherwise we will
            // not have any event listeners on the socket to receive the echoed message.
            if let Some(e) = ws.events().err() {
                console_log!("Error calling ws events from relay {relay}: {e:?}");
                return Err(e.into());
            }

            if let Some(e) = ws.accept().err() {
                console_log!("Error accepting ws from relay {relay}: {e:?}");
                return Err(e.into());
            }

            for message in messages {
                if let Some(e) = ws.send_with_str(message.as_json()).err() {
                    console_log!("Error sending event to relay {relay}: {e:?}")
                }
            }

            if let Some(_e) = ws.close::<String>(None, None).err() {
                console_log!("Error websocket to relay {relay}")
            }
        }
        Err(e) => {
            console_log!("Error connecting to relay {relay}: {e:?}")
        }
    };

    Ok(())
}

pub async fn send_nostr_events(events: Vec<Event>, part: u32) -> Result<Vec<EventId>, Error> {
    let messages: Vec<ClientMessage> = events
        .iter()
        .map(|e| ClientMessage::new_event(e.clone()))
        .collect();

    // pull in the relays from nostr watch list
    let cache = Cache::default();
    let relays = if let Some(mut resp) = cache.get(RELAY_LIST_URL, true).await? {
        console_log!("cache hit for relays");
        match resp.json::<Vec<String>>().await {
            Ok(r) => r,
            Err(_) => RELAYS.iter().map(|x| x.to_string()).collect(),
        }
    } else {
        console_log!("no cache hit for relays");
        match Fetch::Url("https://api.nostr.watch/v1/online".parse().unwrap())
            .send()
            .await
        {
            Ok(mut nostr_resp) => {
                console_log!("retrieved online relay list");
                match nostr_resp.json::<Vec<String>>().await {
                    Ok(r) => {
                        let mut resp = Response::from_json(&r)?;

                        // Cache API respects Cache-Control headers. Setting s-max-age to 10
                        // will limit the response to be in cache for 10 seconds max
                        resp.headers_mut().set("cache-control", "s-maxage=1800")?;
                        cache.put(RELAY_LIST_URL, resp.cloned()?).await?;
                        match resp.json::<Vec<String>>().await {
                            Ok(r) => r,
                            Err(e) => {
                                console_log!("could not parse nostr relay list json: {}", e);
                                RELAYS.iter().map(|x| x.to_string()).collect()
                            }
                        }
                    }
                    Err(e) => {
                        console_log!("could not parse nostr relay list response: {}", e);
                        RELAYS.iter().map(|x| x.to_string()).collect()
                    }
                }
            }
            Err(e) => {
                console_log!("could not retrieve relay list: {}", e);
                RELAYS.iter().map(|x| x.to_string()).collect()
            }
        }
    };
    // find range of elements for this part
    let sub_relays = get_sub_vec_range(relays, find_range_from_part(part));
    let mut futures = Vec::new();
    for relay in sub_relays.iter() {
        let fut = send_event_to_relay(messages.clone(), relay);
        futures.push(fut);
    }
    let combined_futures = futures::future::join_all(futures);
    let sleep = delay(120_000);
    pin_mut!(combined_futures);
    pin_mut!(sleep);
    futures::future::select(combined_futures, sleep).await;
    Ok(events.iter().map(|e| e.id).collect())
}

fn get_sub_vec_range(original: Vec<String>, range: (usize, usize)) -> Vec<String> {
    let len = original.len();
    if range.0 >= len {
        return vec![];
    }
    let end = if range.1 >= len { len - 1 } else { range.1 };
    original[range.0..end].to_vec()
}

fn find_range_from_part(part: u32) -> (usize, usize) {
    let start = 48 * part;
    let end = start + 47;
    (start as usize, end as usize)
}

async fn delay(delay: u64) {
    let delay: Delay = Duration::from_millis(delay).into();
    delay.await;
    console_log!("time delay hit, stopping...");
}
