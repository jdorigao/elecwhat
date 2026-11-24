// Replaces src-web/preload.js (Electron's contextBridge) -- runs before the page's own scripts.
//
// `window.ipc` can't be used here: wry defines it (frozen) for Tauri's own IPC.
// The bridge is therefore `window.ew`; same methods as the old `window.ipc`.
(() => {
  if (window.ew) return;

  const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args ?? {});

  window.ew = {
    debug: false,
    notifyEv: () => invoke("restore"),
    restore: () => invoke("restore"),
    open: (url) => invoke("open_url", { url }),
    stateGet: (name) => invoke("state_get", { name }),
    windowToggle: () => invoke("window_toggle"),
  };

  // --- notifications ------------------------------------------------------
  // WebKitGTK doesn't show desktop notifications for the Web Notification API, so
  // it is replaced by a shim that asks the Rust side (notify-rust / D-Bus).
  // A click on the notification is sent back to the page (window.__ewNotifClick),
  // which is how WhatsApp Web knows to open the right chat.
  const prefixPromise = invoke("state_get", { name: "notifPrefix" }).catch(() => "");
  const live = new Map();
  let seq = 0;

  class EwNotification extends EventTarget {
    static permission = "granted";
    static maxActions = 0;
    static requestPermission(cb) {
      cb?.("granted");
      return Promise.resolve("granted");
    }

    constructor(title, options = {}) {
      super();
      this.title = String(title);
      this.body = options.body ?? "";
      this.tag = options.tag ?? "";
      this.data = options.data ?? null;
      this.icon = options.icon ?? "";
      this.onclick = this.onclose = this.onerror = this.onshow = null;
      this._id = ++seq;
      live.set(this._id, this);
      prefixPromise.then((prefix) =>
        invoke("notify", { id: this._id, title: `${prefix}${this.title}`, body: String(this.body) }),
      );
      setTimeout(() => this._fire("show"), 0);
    }

    _fire(type) {
      const ev = new Event(type);
      this[`on${type}`]?.(ev);
      this.dispatchEvent(ev);
    }

    close() {
      live.delete(this._id);
    }
  }
  window.Notification = EwNotification;
  window.__ewNotifClick = (id) => {
    const n = live.get(id);
    if (n) {
      live.delete(id);
      n._fire("click");
    }
  };

  // --- tray icon / unread count --------------------------------------------
  // Electron had `page-favicon-updated`; WebKit doesn't expose it, so watch <link rel=icon>.
  let lastIcon = null;
  function reportFavicon() {
    const links = document.querySelectorAll('link[rel~="icon"]');
    const href = links[links.length - 1]?.href;
    if (href && href !== lastIcon) {
      lastIcon = href;
      invoke("favicon_updated", { url: href }).catch(() => {});
    }
  }

  document.addEventListener("DOMContentLoaded", () => {
    invoke("page_ready").catch(() => {});
    reportFavicon();
    new MutationObserver(reportFavicon).observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["href"],
    });
  });

  // --- menu bar auto-hide: tap Alt to toggle (like Electron's autoHideMenuBar) -------
  let altAlone = false;
  let autoHide = false;
  invoke("state_get", { name: "menuAutoHide" }).then((v) => (autoHide = !!v)).catch(() => {});
  addEventListener(
    "keydown",
    (ev) => {
      altAlone = ev.key === "Alt" && !ev.repeat && !ev.ctrlKey && !ev.shiftKey && !ev.metaKey;
    },
    true,
  );
  addEventListener(
    "keyup",
    (ev) => {
      if (ev.key === "Alt" && altAlone && autoHide) invoke("menu_toggle").catch(() => {});
      altAlone = false;
    },
    true,
  );
})();
