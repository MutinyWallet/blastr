use crate::nostr::get_nip11_response;
use crate::nostr::NOSTR_QUEUE_10;
use crate::nostr::NOSTR_QUEUE_7;
use crate::nostr::NOSTR_QUEUE_8;
use crate::nostr::NOSTR_QUEUE_9;
pub(crate) use crate::nostr::{
    try_queue_event, NOSTR_QUEUE, NOSTR_QUEUE_2, NOSTR_QUEUE_3, NOSTR_QUEUE_4, NOSTR_QUEUE_5,
    NOSTR_QUEUE_6,
};
use ::nostr::{ClientMessage, Event, RelayMessage};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use worker::*;

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

                                // broadcast it to all queues
                                let nostr_queues = vec![
                                    ctx.env.queue(NOSTR_QUEUE).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_2).expect("get queue"),
                                    ctx.env.queue(NOSTR_QUEUE_3).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_4).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_5).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_6).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_7).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_8).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_9).expect("get queue"),
                                    //ctx.env.queue(NOSTR_QUEUE_10).expect("get queue"),
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

            // For websocket compatibility
            let pair = WebSocketPair::new()?;
            let server = pair.server;
            server.accept()?;
            console_log!("accepted websocket, about to spawn event stream");
            wasm_bindgen_futures::spawn_local(async move {
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

                                        // broadcast it to all queues
                                        let nostr_queues = vec![
                                            ctx.env.queue(NOSTR_QUEUE).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_2).expect("get queue"),
                                            ctx.env.queue(NOSTR_QUEUE_3).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_4).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_5).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_6).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_7).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_8).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_9).expect("get queue"),
                                            //ctx.env.queue(NOSTR_QUEUE_10).expect("get queue"),
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
                                        subscription_id, ..
                                    } => {
                                        console_log!("ignoring nostr subscription request");
                                        let relay_msg = RelayMessage::new_eose(subscription_id);
                                        server
                                            .send_with_str(&relay_msg.as_json())
                                            .expect("failed to send response");
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
