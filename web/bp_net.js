"use strict";

// WebSocket transport for src/net.rs (wasm). Loaded after mq_js_bundle.js, before load("bomberpengu.wasm").
(function () {
    const KIND_MSG = 1, KIND_CLOSED = 2;
    const HEARTBEAT_MS = 15000;
    const MAX_FRAME_BYTES = 65536;
    const MAX_QUEUE = 256;
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
        if (window.BP_DEFAULT_SERVER) return window.BP_DEFAULT_SERVER;
        const tls = location.protocol === "https:";
        return (tls ? "wss://" + location.host : "ws://" + location.hostname + ":6900");
    }

    function register_plugin(importObject) {
        const env = importObject.env;

        env.bp_ws_open = function (ptr, len) {
            const id = nextId++;
            const s = { ws: null, queue: [], pending: [], opened: false, beat: 0, timer: 0 };
            socks.set(id, s);
            const closed = (why) => {
                clearInterval(s.beat);
                clearTimeout(s.timer);
                s.pending.length = 0;
                if (!s.done) {
                    s.done = true;
                    s.queue.push({ kind: KIND_CLOSED, data: enc.encode(why) });
                }
            };
            s.closed = closed;
            try {
                const ws = new WebSocket(read(ptr, len));
                s.ws = ws;
                ws.onopen = () => {
                    if (s.done) return;
                    s.opened = true;
                    for (const text of s.pending) ws.send(text);
                    s.pending.length = 0;
                    s.beat = setInterval(() => { if (ws.readyState === WebSocket.OPEN) ws.send("<beat/>"); }, HEARTBEAT_MS);
                };
                ws.onmessage = (e) => {
                    if (s.done || typeof e.data !== "string") return;
                    clearTimeout(s.timer);
                    if (e.data.length > MAX_FRAME_BYTES || s.queue.length >= MAX_QUEUE) {
                        s.queue.length = 0;
                        closed("Servernachrichten zu gross");
                        ws.close();
                        return;
                    }
                    const data = enc.encode(e.data);
                    if (data.length > MAX_FRAME_BYTES) {
                        s.queue.length = 0;
                        closed("Servernachrichten zu gross");
                        ws.close();
                        return;
                    }
                    s.queue.push({ kind: KIND_MSG, data });
                };
                ws.onclose = () => closed(s.opened ? "Verbindung getrennt" : "Verbindung fehlgeschlagen");
                s.timer = setTimeout(() => {
                    closed("Verbindung fehlgeschlagen");
                    ws.close();
                }, 10000);
            } catch (e) {
                closed("Ungueltige Serveradresse");
            }
            return id;
        };

        env.bp_ws_send = function (id, ptr, len) {
            const s = socks.get(id);
            if (!s || !s.ws || s.done) return;
            if (len > MAX_FRAME_BYTES) {
                s.closed("Nachricht zu gross");
                s.ws.close();
                return;
            }
            const text = read(ptr, len);
            if (s.ws.readyState === WebSocket.OPEN) {
                if (s.ws.bufferedAmount + len > MAX_FRAME_BYTES * MAX_QUEUE) {
                    s.closed("Zu viele Nachrichten");
                    s.ws.close();
                } else {
                    s.ws.send(text);
                }
            } else if (s.ws.readyState === WebSocket.CONNECTING) {
                if (s.pending.length >= MAX_QUEUE) {
                    s.closed("Zu viele Nachrichten");
                    s.ws.close();
                } else {
                    s.pending.push(text);
                }
            }
        };

        env.bp_ws_close = function (id) {
            const s = socks.get(id);
            if (!s) return;
            clearInterval(s.beat);
            clearTimeout(s.timer);
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
