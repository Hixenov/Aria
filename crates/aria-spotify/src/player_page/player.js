// Aria's Spotify player page: runs Spotify's Web Playback SDK as the device "Aria".
// It keeps the session and the token in variables only, stores nothing and writes no log.
"use strict";
(function () {
  const base = location.pathname;
  const tab = new URLSearchParams(location.search).get("tab") === "1";
  let strings = {};
  let session = null;
  let player = null;
  let closing = false;

  // Used when Aria's texts can't be had (a page opened from disk, a reloaded page), so the
  // window always says what is going on.
  const english = {
    "player-page-title": "Aria — Spotify",
    "player-page-playing":
      "Aria is playing Spotify in this window. You can minimize it; closing it stops Spotify in Aria.",
    "player-page-starting": "Starting…",
    "player-page-ready": "Ready",
    "player-page-failed": "Spotify couldn't start here. Aria says why.",
    "player-page-start": "Start",
    "player-page-reload": "Close this window and press play in Aria.",
  };
  const byId = (id) => document.getElementById(id);
  const text = (key) => strings[key] || english[key] || "";
  const say = (key) => {
    byId("status").textContent = text(key);
  };
  const pause = (ms) => new Promise((done) => setTimeout(done, ms));
  const withSession = () => ({ "X-Aria-Session": session });

  function close() {
    if (closing) {
      return;
    }
    closing = true;
    if (player) {
      player.disconnect();
    }
    window.close();
  }

  function post(body) {
    return fetch(base + "event", {
      method: "POST",
      headers: Object.assign({ "Content-Type": "application/json" }, withSession()),
      body: JSON.stringify(body),
    })
      .then((answer) => answer.json())
      .then((answer) => {
        if (answer && answer.close) {
          close();
        }
      })
      .catch(() => {});
  }

  async function waitForClose() {
    while (!closing) {
      try {
        const answer = await (await fetch(base + "wait", { headers: withSession() })).json();
        if (answer && answer.close) {
          close();
          return;
        }
      } catch (e) {
        await pause(1000);
      }
    }
  }

  async function begin() {
    try {
      strings = await (await fetch(base + "strings")).json();
    } catch (e) {
      strings = {};
    }
    const title = text("player-page-title");
    document.title = title;
    byId("title").textContent = title;
    byId("playing").textContent = text("player-page-playing");
    say("player-page-starting");
    try {
      const answer = await fetch(base + "session", { method: "POST" });
      if (!answer.ok) {
        throw new Error("no session");
      }
      session = (await answer.json()).session;
    } catch (e) {
      say("player-page-reload");
      return false;
    }
    setInterval(() => {
      if (!closing) {
        post({ type: "alive" });
      }
    }, 1000);
    waitForClose();
    return true;
  }

  const ready = begin();

  function token(callback) {
    fetch(base + "token", { headers: withSession() })
      .then((answer) => (answer.ok ? answer.json() : Promise.reject(new Error("no token"))))
      .then((answer) => callback(answer.access_token))
      .catch(() => {});
  }

  window.onSpotifyWebPlaybackSDKReady = async function () {
    if (!(await ready)) {
      return;
    }
    player = new Spotify.Player({ name: "Aria", getOAuthToken: token, volume: 0.5 });
    player.addListener("ready", ({ device_id }) => {
      say("player-page-ready");
      post({ type: "ready", device_id: device_id });
    });
    player.addListener("not_ready", ({ device_id }) => {
      post({ type: "not_ready", device_id: device_id });
    });
    const errors = [
      ["initialization_error", "initialization"],
      ["authentication_error", "authentication"],
      ["account_error", "account"],
      ["playback_error", "playback"],
    ];
    for (const [name, kind] of errors) {
      player.addListener(name, ({ message }) => {
        say("player-page-failed");
        post({ type: "error", kind: kind, message: String(message || "") });
      });
    }
    player.addListener("autoplay_failed", () => {
      post({ type: "autoplay_failed" });
    });
    if (tab) {
      const start = byId("start");
      start.textContent = text("player-page-start");
      start.hidden = false;
      start.addEventListener("click", () => {
        start.hidden = true;
        player.activateElement();
        player.connect();
      });
    } else {
      player.connect();
    }
  };
})();
