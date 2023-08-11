use std::collections::HashMap;

use ::nostr::{Event, Kind, RelayMessage};
use nostr::{EventId, Tag, TagKind, Timestamp};
use serde::Deserialize;
use serde_json::Value;
use wasm_bindgen::prelude::JsValue;
use worker::D1Database;
use worker::*;

#[derive(Debug, Deserialize)]
struct EventRow {
    id: EventId,
    pubkey: nostr::secp256k1::XOnlyPublicKey,
    created_at: Timestamp,
    kind: Kind,
    tags: Option<Vec<Tag>>,
    content: String,
    sig: nostr::secp256k1::schnorr::Signature,
}

#[derive(Deserialize)]
struct TagRow {
    event_id: EventId,
    name: String,
    value: String,
}

pub async fn get_nwc_events(keys: &[String], kind: Kind, db: &D1Database) -> Result<Vec<Event>> {
    // Determine the event kind
    match kind {
        Kind::WalletConnectResponse => (),
        Kind::WalletConnectRequest => (),
        _ => return Ok(vec![]), // skip other event types
    };

    console_log!("querying for ({keys:?}) and {}", kind.as_u32());

    // Query for the events first, without the tags
    let placeholders: String = keys.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let query_str = format!(
        r#"
        SELECT * FROM event
        WHERE pubkey IN ({}) AND kind = ? AND deleted = 0
        "#,
        placeholders
    );
    let mut stmt = db.prepare(&query_str);
    let mut bindings = Vec::with_capacity(keys.len() + 1); // +1 for the kind afterwards
    for key in keys.iter() {
        bindings.push(JsValue::from_str(key));
    }
    bindings.push(JsValue::from_f64(kind.as_u32() as f64));
    stmt = stmt.bind(&bindings)?;

    let result = stmt.all().await.map_err(|e| {
        console_log!("Failed to fetch nwc events: {}", e);
        format!("Failed to fetch nwc events: {}", e)
    })?;

    let mut events: Vec<Event> = result
        .results::<Value>()?
        .iter()
        .map(|row| {
            let e: EventRow = serde_json::from_value(row.clone()).map_err(|e| {
                console_log!("failed to parse event: {}", e);
                worker::Error::from(format!(
                    "Failed to deserialize event from row ({}): {}",
                    row, e
                ))
            })?;
            Ok(Event {
                id: e.id,
                pubkey: e.pubkey,
                created_at: e.created_at,
                kind: e.kind,
                tags: e.tags.unwrap_or_default(),
                content: e.content,
                sig: e.sig,
            })
        })
        .collect::<Result<Vec<Event>>>()?;

    // Now get all the tags for all the events found
    let event_ids: Vec<EventId> = events.iter().map(|e| e.id).collect();
    let placeholders: String = event_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let tag_query_str = format!(
        r#"
        SELECT event_id, name, value FROM tag
        WHERE event_id IN ({})
        ORDER BY id ASC
        "#,
        placeholders
    );
    let mut tag_stmt = db.prepare(&tag_query_str);
    let bindings: Vec<JsValue> = event_ids
        .iter()
        .map(|id| JsValue::from_str(&id.to_string()))
        .collect();
    tag_stmt = tag_stmt.bind(&bindings)?;

    let tag_result = tag_stmt
        .all()
        .await
        .map_err(|e| format!("Failed to fetch tags: {}", e))?;

    let tags: Vec<TagRow> = tag_result.results::<TagRow>()?;
    let mut tags_map: HashMap<EventId, Vec<Tag>> = HashMap::new();

    for tag_row in tags {
        if let Ok(tag) = Tag::parse(vec![tag_row.name, tag_row.value]) {
            tags_map
                .entry(tag_row.event_id)
                .or_insert_with(Vec::new)
                .push(tag);
        }
    }

    for event in &mut events {
        if let Some(tags) = tags_map.remove(&event.id) {
            event.tags.extend(tags);
        }
    }

    // Tag ordering could screw up signature, though it shouldn't matter
    // for NWC messages because those should only have one tag.
    // Also we insert tags in order and do an ORDER BY id so it should be fine.
    events.retain(|event| match event.verify() {
        Ok(_) => true,
        Err(e) => {
            console_log!("Verification failed for event with id {}: {}", event.id, e);
            false
        }
    });

    Ok(events)
}

pub async fn handle_nwc_event(event: Event, db: &D1Database) -> Result<Option<RelayMessage>> {
    // Determine the event kind
    match event.kind {
        Kind::WalletConnectResponse => (),
        Kind::WalletConnectRequest => (),
        _ => return Ok(None), // skip other event types
    };

    // Create the main event insertion query.
    let event_insert_query = worker::query!(
        db,
        r#"
        INSERT OR IGNORE INTO event (id, created_at, pubkey, kind, content, sig)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
        &event.id,
        &event.created_at,
        &event.pubkey,
        &event.kind,
        &event.content,
        &event.sig
    )?;

    // Create a vector of tag insertion queries.
    let mut tag_insert_queries: Vec<_> = event
        .tags
        .iter()
        .map(|tag| {
            worker::query!(
                db,
                r#"
                INSERT OR IGNORE INTO tag (event_id, name, value)
                VALUES (?, ?, ?)
                "#,
                &event.id,
                &tag.kind().to_string(),
                &tag.as_vec().get(1)
            )
            .expect("should compile query")
        })
        .collect();

    // Combine the main event and tag insertion queries.
    let mut batch_queries = vec![event_insert_query];
    batch_queries.append(&mut tag_insert_queries);

    // Run the batch queries.
    let mut results = db.batch(batch_queries).await?.into_iter();

    // Check the result of the main event insertion.
    if let Some(error_msg) = results.next().and_then(|res| res.error()) {
        console_log!("error saving nwc event to event table: {}", error_msg);
        let relay_msg = RelayMessage::new_ok(event.id, false, &error_msg);
        return Ok(Some(relay_msg));
    }

    // Check the results for the tag insertions.
    for tag_insert_result in results {
        if let Some(error_msg) = tag_insert_result.error() {
            console_log!("error saving tag to tag table: {}", error_msg);
            let relay_msg = RelayMessage::new_ok(event.id, false, &error_msg);
            return Ok(Some(relay_msg));
        }
    }

    let relay_msg = RelayMessage::new_ok(event.id, true, "");
    Ok(Some(relay_msg))
}

/// When a NWC request has been fulfilled, soft delete the request from the database
pub async fn delete_nwc_request(event: Event, db: &D1Database) -> Result<()> {
    // Filter only relevant events
    match event.kind {
        Kind::WalletConnectResponse => (),
        _ => return Ok(()), // skip other event types
    };

    let p_tag = event.tags.iter().find(|t| t.kind() == TagKind::P).cloned();
    let e_tag = event.tags.iter().find(|t| t.kind() == TagKind::E).cloned();

    if let Some(Tag::PubKey(pubkey, ..)) = p_tag {
        if let Some(Tag::Event(event_id, ..)) = e_tag {
            // Soft delete the event based on pubkey and event_id
            match worker::query!(
                db,
                "UPDATE event SET deleted = 1 WHERE pubkey = ? AND id = ?",
                &pubkey.to_string(),
                &event_id
            )?
            .run()
            .await
            {
                Ok(_) => (),
                Err(e) => {
                    console_log!("error soft deleting nwc event from database: {e}");
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

/// When a NWC response has been fulfilled, soft delete the response from the database
pub async fn delete_nwc_response(event: &Event, db: &D1Database) -> Result<()> {
    // Filter only relevant events
    match event.kind {
        Kind::WalletConnectResponse => (),
        _ => return Ok(()), // skip other event types
    };

    // Soft delete the event based on pubkey and id
    match worker::query!(
        db,
        "UPDATE event SET deleted = 1 WHERE pubkey = ? AND id = ?",
        &event.pubkey.to_string(),
        &event.id
    )?
    .run()
    .await
    {
        Ok(_) => console_log!("soft deleted nwc response event: {}", event.id),
        Err(e) => {
            console_log!("error soft deleting nwc event from database: {e}");
            return Ok(());
        }
    }

    Ok(())
}
