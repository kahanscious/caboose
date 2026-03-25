export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === '/health') {
      return new Response('ok');
    }

    if (url.pathname !== '/relay') {
      return new Response('Not found', { status: 404 });
    }

    const room = url.searchParams.get('room');
    if (!room) {
      return new Response('Missing room parameter', { status: 400 });
    }

    // Route to Durable Object by room ID
    const id = env.RELAY_ROOM.idFromName(room);
    const obj = env.RELAY_ROOM.get(id);
    return obj.fetch(request);
  }
};

export class RelayRoom {
  constructor(state) {
    this.state = state;
    this.clients = new Map(); // ws -> metadata
  }

  async fetch(request) {
    if (this.clients.size >= 2) {
      return new Response('Room full', { status: 429 });
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    this.clients.set(server, { ready: false });

    server.accept();

    server.addEventListener('message', (event) => {
      // Forward to all other clients in the room
      for (const [ws] of this.clients) {
        if (ws !== server && ws.readyState === WebSocket.READY_STATE_OPEN) {
          ws.send(event.data);
        }
      }
    });

    server.addEventListener('close', () => {
      this.clients.delete(server);
    });

    server.addEventListener('error', () => {
      this.clients.delete(server);
    });

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }
}
