use crate::nostr::NOSTR_QUEUE_8;
use crate::nostr::NOSTR_QUEUE_9;
pub(crate) use crate::nostr::{
    try_queue_event, NOSTR_QUEUE, NOSTR_QUEUE_2, NOSTR_QUEUE_3, NOSTR_QUEUE_4, NOSTR_QUEUE_5,
    NOSTR_QUEUE_6,
};
use crate::{db::delete_nwc_request, nostr::NOSTR_QUEUE_10};
use crate::{db::get_nwc_events, nostr::NOSTR_QUEUE_7};
use crate::{db::handle_nwc_event, nostr::get_nip11_response};
use ::nostr::{ClientMessage, Event, EventId, Filter, Kind, RelayMessage, SubscriptionId, Tag};
use futures::StreamExt;
use futures_util::lock::Mutex;
use serde::{Deserialize, Serialize};
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use worker::*;

mod db;
mod error;
mod nostr;
mod utils;

fn log_request(req: &Request) {
    console_log!(
        "Incoming Request: {} - [{}]",
        Date::now().to_string(),
        req.path(),
    );
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PublishedNote {
    date: String,
}

/// The list of event kinds that are disallowed for the Nostr relay
/// currently, this is just the NIP-95 event
const DISALLOWED_EVENT_KINDS: [u32; 1] = [1064];

/// Main function for the Cloudflare Worker that triggers off of a HTTP req
#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    log_request(&req);

    // Optionally, get more helpful error messages written to the console in the case of a panic.
    utils::set_panic_hook();

    // Optionally, use the Router to handle matching endpoints, use ":name" placeholders, or "*name"
    // catch-alls to match on specific patterns. Alternatively, use `Router::with_data(D)` to
    // provide arbitrary data that will be accessible in each route via the `ctx.data()` method.
    let router = Router::new();

    // Add as many routes as your Worker needs! Each route will get a `Request` for handling HTTP
    // functionality and a `RouteContext` which you can use to  and get route parameters and
    // Environment bindings like KV Stores, Durable Objects, Secrets, and Variables.
    router
        .post_async("/event", |mut req, ctx| async move {
            // for any adhoc POST event
            match req.text().await {
                Ok(request_text) => {
                    if let Ok(client_msg) = ClientMessage::from_json(request_text) {
                        match client_msg {
                            ClientMessage::Event(event) => {
                                console_log!("got an event from client: {}", event.id);

                                match event.verify() {
                                    Ok(()) => (),
                                    Err(e) => {
                                        console_log!("could not verify event {}: {}", event.id, e);
                                        let relay_msg = RelayMessage::new_ok(
                                            event.id,
                                            false,
                                            "invalid event",
                                        );
                                        return relay_response(relay_msg);
                                    }
                                }

                                // check if disallowed event kind
                                if DISALLOWED_EVENT_KINDS.contains(&event.kind.as_u32()) {
                                    console_log!(
                                        "invalid event kind {}: {}",
                                        event.kind.as_u32(),
                                        event.id
                                    );
                                    let relay_msg = RelayMessage::new_ok(
                                        event.id,
                                        false,
                                        "disallowed event kind",
                                    );
                                    return relay_response(relay_msg);
                                };

                                // check if we've already published it before
                                let published_notes = ctx.kv("PUBLISHED_NOTES")?;
                                if published_notes
                                    .get(event.id.to_string().as_str())
                                    .json::<PublishedNote>()
                                    .await
                                    .ok()
                                    .flatten()
                                    .is_some()
                                {
                                    console_log!("event already published: {}", event.id);
                                    let relay_msg = RelayMessage::new_ok(
                                        event.id,
                                        false,
                                        "event already published",
                                    );
                                    return relay_response(relay_msg);
                                };

                                let db = ctx.d1("DB")?;

                                // if the event is a nostr wallet connect event, we
                                // should save it and not send to other relays.
                                if let Some(relay_msg) =
                                    handle_nwc_event(*event.clone(), &db).await?
                                {
                                    if let Err(e) = delete_nwc_request(*event, &db).await {
                                        console_log!("failed to delete nwc request: {e}");
                                    }
                                    return relay_response(relay_msg);
                                };

                                // broadcast it to all queues
                                let nostr_queues = vec![
                                    ctx.env.queue(NOSTR_QUEUE).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_2).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_3).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_4).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_5).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_6).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_7).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_8).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_9).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_10).expect("get queue"),
                                ];
                                try_queue_event(*event.clone(), nostr_queues).await;
                                console_log!("queued up nostr event: {}", event.id);
                                match published_notes
                                    .put(
                                        event.id.to_string().as_str(),
                                        PublishedNote {
                                            date: Date::now().to_string(),
                                        },
                                    )?
                                    .execute()
                                    .await
                                {
                                    Ok(_) => {
                                        console_log!("saved published note: {}", event.id);
                                        let relay_msg = RelayMessage::new_ok(event.id, true, "");
                                        relay_response(relay_msg)
                                    }
                                    Err(e) => {
                                        console_log!(
                                            "could not save published note: {} - {e:?}",
                                            event.id
                                        );
                                        let relay_msg = RelayMessage::new_ok(
                                            event.id,
                                            false,
                                            "error: could not save published note",
                                        );
                                        relay_response(relay_msg)
                                    }
                                }
                            }
                            _ => {
                                console_log!("ignoring other nostr client message types");
                                Response::error("Only Event types allowed", 400)
                            }
                        }
                    } else {
                        Response::error("Could not parse Client Message", 400)
                    }
                }
                Err(e) => {
                    console_log!("could not get request text: {}", e);
                    Response::error("Could not get request text", 400)
                }
            }
        })
        .get("/", |req, ctx| {
            // NIP 11
            if req.headers().get("Accept").ok().flatten()
                == Some("application/nostr+json".to_string())
            {
                return Response::from_json(&get_nip11_response())?.with_cors(&cors());
            }

            let ctx = Rc::new(ctx);
            // For websocket compatibility
            let pair = WebSocketPair::new()?;
            let server = pair.server;
            server.accept()?;
            console_log!("accepted websocket, about to spawn event stream");
            wasm_bindgen_futures::spawn_local(async move {
                let running_thread = Arc::new(AtomicBool::new(false));
                let new_subscription_req = Arc::new(AtomicBool::new(false));
                let requested_filters = Arc::new(Mutex::new(Filter::new()));
                let mut event_stream = server.events().expect("stream error");
                console_log!("spawned event stream, waiting for first message..");
                while let Some(event) = event_stream.next().await {
                    if let Err(e) = event {
                        console_log!("error parsing some event: {e}");
                        continue;
                    }
                    match event.expect("received error in websocket") {
                        WebsocketEvent::Message(msg) => {
                            if msg.text().is_none() {
                                continue;
                            };
                            if let Ok(client_msg) = ClientMessage::from_json(msg.text().unwrap()) {
                                match client_msg {
                                    ClientMessage::Event(event) => {
                                        console_log!("got an event from client: {}", event.id);
                                        match event.verify() {
                                            Ok(()) => (),
                                            Err(e) => {
                                                console_log!("could not verify event {}: {}", event.id, e);
                                                let relay_msg = RelayMessage::new_ok(
                                                    event.id,
                                                    false,
                                                    "disallowed event kind",
                                                );
                                                server
                                                    .send_with_str(&relay_msg.as_json())
                                                    .expect("failed to send response");
                                                continue;
                                            }
                                        }

                                        // check if disallowed event kind
                                        if DISALLOWED_EVENT_KINDS.contains(&event.kind.as_u32()) {
                                            console_log!(
                                                "invalid event kind {}: {}",
                                                event.kind.as_u32(),
                                                event.id
                                            );
                                            let relay_msg = RelayMessage::new_ok(
                                                event.id,
                                                false,
                                                "disallowed event kind",
                                            );
                                            server
                                                .send_with_str(&relay_msg.as_json())
                                                .expect("failed to send response");
                                            continue;
                                        };

                                        // check if we've already published it before
                                        let published_notes =
                                            ctx.kv("PUBLISHED_NOTES").expect("get kv");
                                        if published_notes
                                            .get(event.id.to_string().as_str())
                                            .json::<PublishedNote>()
                                            .await
                                            .ok()
                                            .flatten()
                                            .is_some()
                                        {
                                            console_log!("event already published: {}", event.id);
                                            let relay_msg = RelayMessage::new_ok(
                                                event.id,
                                                false,
                                                "event already published",
                                            );
                                            server
                                                .send_with_str(&relay_msg.as_json())
                                                .expect("failed to send response");
                                            continue;
                                        };

                                        let db = ctx.d1("DB").expect("should have DB");

                                        // if the event is a nostr wallet connect event, we
                                        // should save it and not send to other relays.
                                        if let Some(response) =
                                            handle_nwc_event(*event.clone(), &db)
                                                .await
                                                .expect("failed to handle nwc event")
                                        {
                                            server
                                                .send_with_str(&response.as_json())
                                                .expect("failed to send response");

                                            if let Err(e) = delete_nwc_request(*event, &db).await {
                                                console_log!("failed to delete nwc request: {e}");
                                            }

                                            continue;
                                        };

                                        // broadcast it to all queues
                                        let nostr_queues = vec![
                                            ctx.env.queue(NOSTR_QUEUE).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_2).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_3).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_4).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_5).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_6).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_7).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_8).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_9).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_10).expect("get queue"),
                                        ];
                                        try_queue_event(*event.clone(), nostr_queues).await;
                                        console_log!("queued up nostr event: {}", event.id);
                                        match published_notes
                                            .put(
                                                event.id.to_string().as_str(),
                                                PublishedNote {
                                                    date: Date::now().to_string(),
                                                },
                                            )
                                            .expect("saved note")
                                            .execute()
                                            .await
                                        {
                                            Ok(_) => {
                                                console_log!("saved published note: {}", event.id);
                                            }
                                            Err(e) => {
                                                console_log!(
                                                    "could not save published note: {} - {e:?}",
                                                    event.id
                                                );
                                            }
                                        }

                                        let relay_msg = RelayMessage::new_ok(event.id, true, "");
                                        server
                                            .send_with_str(&relay_msg.as_json())
                                            .expect("failed to send response");
                                    }
                                    ClientMessage::Req {
                                        subscription_id,
                                        filters,
                                    } => {
                                        new_subscription_req.swap(true, Ordering::Relaxed);
                                        console_log!("got a new client request sub: {}, len: {}", subscription_id, filters.len());
                                        // for each filter we handle it every 10 seconds
                                        // by reading storage and sending any new events
                                        // one caveat is that this will send events multiple
                                        // times if they are in multiple filters
                                        let mut valid = false;
                                        for filter in filters {
                                            let valid_nwc = {
                                                // has correct kinds
                                                let kinds = filter.kinds.as_ref();
                                                (
                                                    kinds
                                                        .unwrap_or(&vec![])
                                                        .contains(&Kind::WalletConnectResponse)
                                                        || kinds.unwrap_or(&vec![])
                                                        .contains(&Kind::WalletConnectRequest)
                                                ) &&
                                                    // has authors or pubkeys
                                                    !filter.authors.as_ref().unwrap_or(&vec![]).is_empty() ||
                                                    !filter.pubkeys.as_ref().unwrap_or(&vec![]).is_empty()
                                            };

                                            if valid_nwc {
                                                let mut master_guard = requested_filters.lock().await;
                                                let master_filter = master_guard.deref_mut();
                                                // now add the new filters to the main filter
                                                // object. This is a bit of a hack but we only
                                                // check certain sub filters for NWC.
                                                combine_filters(master_filter, &filter);
                                                console_log!("New filter count: {}", master_filter.pubkeys.as_ref().map_or(0, Vec::len));
                                                valid = true;
                                            }
                                        }

                                        // only spin up a new one if there's not a
                                        // spawn_local already going with filters
                                        // when other filters are added in, it should
                                        // be picked up in the master filter
                                        let mut sent_event_count = 0;
                                        if !running_thread.load(Ordering::Relaxed) && valid {
                                            // set running thread to true
                                            running_thread.swap(true, Ordering::Relaxed);

                                            let db = ctx.d1("DB").expect("should have DB");
                                            let sub_id = subscription_id.clone();
                                            let server_clone = server.clone();
                                            let master_clone = requested_filters.clone();
                                            let new_subscription_req_clone = new_subscription_req.clone();
                                            wasm_bindgen_futures::spawn_local(async move {
                                                let mut sent_events = vec![];
                                                loop {
                                                    let master = master_clone.lock().await;
                                                    console_log!("Checking filters: {}", master.pubkeys.as_ref().map_or(0, Vec::len));
                                                    match handle_filter(
                                                        &sent_events,
                                                        sub_id.clone(),
                                                        master.clone(),
                                                        &server_clone,
                                                        &db,
                                                    ).await
                                                    {
                                                        Ok(new_event_ids) => {
                                                            // add new events to sent events
                                                            sent_events.extend(new_event_ids);
                                                            // send EOSE if necessary
                                                            if new_subscription_req_clone.load(Ordering::Relaxed) || sent_event_count != sent_events.len() {
                                                                let relay_msg = RelayMessage::new_eose(sub_id.clone());
                                                                server_clone
                                                                    .send_with_str(relay_msg.as_json())
                                                                    .expect("failed to send response");
                                                                sent_event_count = sent_events.len();
                                                                new_subscription_req_clone.swap(false, Ordering::Relaxed);
                                                            }
                                                        }
                                                        Err(e) => console_log!(
                                                            "error handling filter: {e}"
                                                        ),
                                                    }
                                                    drop(master);
                                                    utils::delay(5_000).await;
                                                }
                                            });
                                        } else if !valid {
                                            // if not a nwc filter, we just send EOSE
                                            let relay_msg = RelayMessage::new_eose(subscription_id);
                                            server
                                                .send_with_str(relay_msg.as_json())
                                                .expect("failed to send response");
                                        }
                                    }
                                    _ => {
                                        console_log!("ignoring other nostr client message types");
                                    }
                                }
                            }
                        }
                        WebsocketEvent::Close(_) => {
                            console_log!("closing");
                            break;
                        }
                    }
                }
            });
            Response::from_websocket(pair.client)
        })
        .get("/favicon.ico", |_, _| {
            let bytes: Vec<u8> = include_bytes!("../static/favicon.ico").to_vec();
            Response::from_bytes(bytes)?.with_cors(&cors())
        })
        .options("/*catchall", |_, _| empty_response())
        .run(req, env)
        .await
}

/// Main function for the Cloudflare Worker that triggers off the nostr event queue
#[event(queue)]
pub async fn main(message_batch: MessageBatch<Event>, _env: Env, _ctx: Context) -> Result<()> {
    // Deserialize the message batch
    let messages: Vec<Message<Event>> = message_batch.messages()?;
    let mut events: Vec<Event> = messages.iter().map(|m| m.body.clone()).collect();
    events.sort();
    events.dedup();

    let part = queue_number(message_batch.queue().as_str())?;
    match nostr::send_nostr_events(events, part).await {
        Ok(event_ids) => {
            for event_id in event_ids {
                console_log!("Sent nostr event: {}", event_id)
            }
        }
        Err(error::Error::WorkerError(e)) => {
            console_log!("worker error: {e}");
        }
    }

    Ok(())
}

pub fn queue_number(batch_name: &str) -> Result<u32> {
    match batch_name {
        NOSTR_QUEUE => Ok(0),
        NOSTR_QUEUE_2 => Ok(1),
        NOSTR_QUEUE_3 => Ok(2),
        NOSTR_QUEUE_4 => Ok(3),
        NOSTR_QUEUE_5 => Ok(4),
        NOSTR_QUEUE_6 => Ok(5),
        NOSTR_QUEUE_7 => Ok(6),
        NOSTR_QUEUE_8 => Ok(7),
        NOSTR_QUEUE_9 => Ok(8),
        NOSTR_QUEUE_10 => Ok(9),
        _ => Err("unexpected queue".into()),
    }
}

/// if the user requests a NWC event, we have those stored,
/// we should send them to the user
pub async fn handle_filter(
    sent_events: &[EventId],
    subscription_id: SubscriptionId,
    filter: Filter,
    server: &WebSocket,
    db: &D1Database,
) -> Result<Vec<EventId>> {
    let mut events = vec![];
    // get all authors and pubkeys
    let mut keys = filter.authors.unwrap_or_default();
    keys.extend(
        filter
            .pubkeys
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string()),
    );

    if filter
        .kinds
        .clone()
        .unwrap_or_default()
        .contains(&Kind::WalletConnectRequest)
    {
        let mut found_events = get_nwc_events(&keys, Kind::WalletConnectRequest, db)
            .await
            .unwrap_or_default();

        // filter out events that have already been sent
        found_events.retain(|e| !sent_events.contains(&e.id));

        events.extend(found_events);
    }

    if filter
        .kinds
        .unwrap_or_default()
        .contains(&Kind::WalletConnectResponse)
    {
        let mut found_events = get_nwc_events(&keys, Kind::WalletConnectResponse, db)
            .await
            .unwrap_or_default();

        // filter out events that have already been sent
        found_events.retain(|e| !sent_events.contains(&e.id));

        events.extend(found_events);
    }

    // if the filter is only requesting replies to a certain event filter to only those
    if let Some(event_ids) = filter.events {
        events.retain(|event| {
            event
                .tags
                .iter()
                .any(|t| matches!(t, Tag::Event(id, _, _) if event_ids.contains(id)))
        })
    }

    if events.is_empty() {
        return Ok(vec![]);
    }

    if !events.is_empty() {
        // sort and dedup events
        events.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        events.dedup();

        // send all found events to the user
        for event in events.clone() {
            console_log!("sending event to client: {}", &event.id);
            let relay_msg = RelayMessage::new_event(subscription_id.clone(), event);
            server
                .send_with_str(&relay_msg.as_json())
                .expect("failed to send response");
        }
    }

    let sent_event_ids: Vec<EventId> = events.into_iter().map(|e| e.id).collect();
    Ok(sent_event_ids)
}

// Helper function to extend a vector without duplicates
fn extend_without_duplicates<T: PartialEq + Clone>(master: &mut Vec<T>, new: &Vec<T>) {
    for item in new {
        if !master.contains(item) {
            master.push(item.clone());
        }
    }
}

fn combine_filters(master_filter: &mut Filter, new_filter: &Filter) {
    // Check and extend for IDs
    if let Some(vec) = &new_filter.ids {
        let master_vec = master_filter.ids.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for authors
    if let Some(vec) = &new_filter.authors {
        let master_vec = master_filter.authors.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for kinds
    if let Some(vec) = &new_filter.kinds {
        let master_vec = master_filter.kinds.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for events
    if let Some(vec) = &new_filter.events {
        let master_vec = master_filter.events.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for pubkeys
    if let Some(vec) = &new_filter.pubkeys {
        let master_vec = master_filter.pubkeys.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for hashtags
    if let Some(vec) = &new_filter.hashtags {
        let master_vec = master_filter.hashtags.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for references
    if let Some(vec) = &new_filter.references {
        let master_vec = master_filter.references.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }

    // Check and extend for identifiers
    if let Some(vec) = &new_filter.identifiers {
        let master_vec = master_filter.identifiers.get_or_insert_with(Vec::new);
        extend_without_duplicates(master_vec, vec);
    }
}

fn relay_response(msg: RelayMessage) -> worker::Result<Response> {
    Response::from_json(&msg)?.with_cors(&cors())
}

fn empty_response() -> worker::Result<Response> {
    Response::empty()?.with_cors(&cors())
}

fn cors() -> Cors {
    Cors::new()
        .with_credentials(true)
        .with_origins(vec!["*"])
        .with_allowed_headers(vec!["Content-Type"])
        .with_methods(Method::all())
}
