"use strict";

// WebSocket transport for src/net.rs (wasm). Loaded after mq_js_bundle.js, before load("bomberpengu.wasm").
(function () {
    const KIND_MSG = 1, KIND_CLOSED = 2;
    const HEARTBEAT_MS = 15000;
    const socks = new Map();
    let nextId = 1;
    const dec = new TextDecoder();
    const enc = new TextEncoder();

    function read(ptr, len) {
        return dec.decode(new Uint8Array(wasm_memory.buffer, ptr, len));
    }

    function pageServer() {
        const q = new URLSearchParams(location.search).get("server");
        if (q) return q;
        const tls = location.protocol === "https:";
        return (tls ? "wss://" + location.host : "ws://" + location.hostname + ":6900");
    }

    function register_plugin(importObject) {
        const env = importObject.env;

        env.bp_ws_open = function (ptr, len) {
            const id = nextId++;
            const s = { ws: null, queue: [], opened: false, beat: 0 };
            socks.set(id, s);
            const closed = (why) => {
                clearInterval(s.beat);
                if (!s.done) {
                    s.done = true;
                    s.queue.push({ kind: KIND_CLOSED, data: enc.encode(why) });
                }
            };
            try {
                const ws = new WebSocket(read(ptr, len));
                s.ws = ws;
                ws.onopen = () => {
                    s.opened = true;
                    s.beat = setInterval(() => { if (ws.readyState === WebSocket.OPEN) ws.send("<beat/>"); }, HEARTBEAT_MS);
                };
                ws.onmessage = (e) => {
                    if (typeof e.data === "string") s.queue.push({ kind: KIND_MSG, data: enc.encode(e.data) });
                };
                ws.onclose = () => closed(s.opened ? "Verbindung getrennt" : "Verbindung fehlgeschlagen");
            } catch (e) {
                closed("Ungueltige Serveradresse");
            }
            return id;
        };

        env.bp_ws_send = function (id, ptr, len) {
            const s = socks.get(id);
            if (!s || !s.ws) return;
            const text = read(ptr, len);
            if (s.ws.readyState === WebSocket.OPEN) {
                s.ws.send(text);
            } else if (s.ws.readyState === WebSocket.CONNECTING) {
                // messages sent before the socket is up (the auth) go out as soon as it opens
                const prev = s.ws.onopen;
                s.ws.onopen = function (ev) { prev(ev); s.ws.send(text); };
            }
        };

        env.bp_ws_close = function (id) {
            const s = socks.get(id);
            if (!s) return;
            clearInterval(s.beat);
            if (s.ws) { s.ws.onclose = null; s.ws.close(); }
            socks.delete(id);
        };

        env.bp_ws_next_kind = function (id) {
            const s = socks.get(id);
            return s && s.queue.length ? s.queue[0].kind : 0;
        };

        env.bp_ws_next_len = function (id) {
            const s = socks.get(id);
            return s && s.queue.length ? s.queue[0].data.length : 0;
        };

        env.bp_ws_take = function (id, dst) {
            const s = socks.get(id);
            if (!s || !s.queue.length) return;
            const ev = s.queue.shift();
            new Uint8Array(wasm_memory.buffer, dst, ev.data.length).set(ev.data);
        };

        env.bp_page_server = function (dst, cap) {
            const bytes = enc.encode(pageServer()).subarray(0, cap);
            new Uint8Array(wasm_memory.buffer, dst, bytes.length).set(bytes);
            return bytes.length;
        };
    }

    miniquad_add_plugin({ register_plugin, name: "bp_net", version: 1 });
})();
