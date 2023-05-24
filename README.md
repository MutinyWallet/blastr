# blastr

A nostr cloudflare workers proxy relay that publishes to all known online relays.

![blastr diagram](./docs/images/blastr-diagram.png)

This takes advantage of the high availabilty of cloudflare serverless workers on the edge that are rust wasm-based with 0ms cold starts. Learn more about [cloudflare workers](https://workers.cloudflare.com/).

This is write only for now and is compatible with nostr clients but also features a simple POST api endpoint at `/event`. All events get queued up to run in batches by another worker that spins up every 30s if there's any events lined up, or once a certain amount of events are queued up.

This will help ensure that your events are broadcasted to as many places as possible.

## Development

With `wrangler`, you can build, test, and deploy your Worker with the following commands:

```sh
# install wrangler if you do not have it yet
$ npm install -g wrangler

# log into cloudflare if you havent before
$ wrangler login

# compiles your project to WebAssembly and will warn of any issues
$ npm run build

# run your Worker in an ideal development workflow (with a local server, file watcher & more)
$ npm run dev

# deploy your Worker globally to the Cloudflare network (update your wrangler.toml file for configuration)
$ npm run deploy
```

### Setup

There's a few cloudflare components that Blastr uses behind the scenes, namely a KV store and multiple queues to distribute the load.

Right now some of these are hardcoded for us since they have to map from the `wrangler.toml` file to the rust codebase. Need a TODO for making this more dynamic.

#### KV store

This doesn't rebroadcast events that have already been broadcasted before. So we have a KV for that.

We also have a KV for storing the NWC requests and responses to get around ephemeral events.

```
wrangler kv:namespace create PUBLISHED_NOTES
wrangler kv:namespace create PUBLISHED_NOTES --preview

wrangler kv:namespace create NWC_REQUESTS
wrangler kv:namespace create NWC_REQUESTS --preview

wrangler kv:namespace create NWC_RESPONSES
wrangler kv:namespace create NWC_RESPONSES --preview
```

#### Queues

```
 wrangler queues create nostr-events-pub-1-b
 wrangler queues create nostr-events-pub-2-b
 wrangler queues create nostr-events-pub-3-b
 wrangler queues create nostr-events-pub-4-b
 wrangler queues create nostr-events-pub-5-b
 wrangler queues create nostr-events-pub-6-b
 wrangler queues create nostr-events-pub-7-b
 wrangler queues create nostr-events-pub-8-b
 wrangler queues create nostr-events-pub-9-b
 wrangler queues create nostr-events-pub-10-b
```

Read the latest `worker` crate documentation here: https://docs.rs/worker

### CICD

There's an example workflow here for publishing on master branch pushes. You need to set `CF_API_TOKEN` in your github repo secrets first.

You also should either remove or configure `wrangler.toml` to point to a custom domain of yours:

```
routes = [
    { pattern = "example.com/about", zone_id = "<YOUR_ZONE_ID>" } # replace with your info
]
```

and any other info in `wrangler.toml` that is custom to you, like the names / id's of queues or kv's.

### WebAssembly

`workers-rs` (the Rust SDK for Cloudflare Workers used in this template) is meant to be executed as compiled WebAssembly, and as such so **must** all the code you write and depend upon. All crates and modules used in Rust-based Workers projects have to compile to the `wasm32-unknown-unknown` triple.

Read more about this on the [`workers-rs`](https://github.com/cloudflare/workers-rs) project README.
