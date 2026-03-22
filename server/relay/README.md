# Caboose Relay

WebSocket frame relay for Caboose Mobile. Pairs a desktop Caboose server and phone client in a room and forwards encrypted frames between them. The relay never inspects message content — all data is encrypted end-to-end.

## How it works

- Desktop connects to `/relay?room=ROOM_ID`
- Phone connects to `/relay?room=ROOM_ID`
- Messages from one client are forwarded to the other
- Max 2 clients per room; additional connections are rejected
- Rooms are destroyed when both clients disconnect

## Deploy

```bash
npm install
npx wrangler deploy
```

## Self-host

Change the `name` in `wrangler.toml` and deploy to your own Cloudflare account:

```bash
npx wrangler login
npx wrangler deploy
```

## Default instance

The default relay is hosted at `relay.trycaboose.dev`.
