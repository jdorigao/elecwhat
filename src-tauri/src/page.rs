//! Page-context script, kept in Rust instead of a separate `.js` file.
//!
//! It is installed as a Tauri initialization script, so it runs before the
//! page's own scripts. It only activates on the configured WhatsApp host.
//!
//! `__EW_*__` placeholders are replaced with JSON literals by [`script`].

const SCRIPT: &str = r#"
(() => {
  const HOST = __EW_HOST__;
  const PREFIX = __EW_PREFIX__;
  const ESC_TOGGLE = __EW_ESC__;
  const MENU_AUTO_HIDE = __EW_MENU__;

  if (location.protocol !== "https:" || location.host !== HOST) return;
  const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args ?? {});

  // --- notifications -------------------------------------------------------
  // WebKitGTK does not turn the Web Notification API into desktop
  // notifications, so WhatsApp's `new Notification(...)` is forwarded to the
  // Rust side (notify-rust / D-Bus). A click is sent back via __ewNotifClick,
  // which is how WhatsApp Web learns which chat to open.
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
      const id = ++seq;
      this._id = id;
      live.set(id, this);
      invoke("notify", {
        id,
        title: `${PREFIX}${this.title}`,
        body: String(this.body),
      }).catch(() => {});
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
  // Electron had `page-favicon-updated`; WebKit exposes no such event, so
  // watch <link rel="icon"> instead.
  let lastIcon = null;
  let watching = false;
  function reportFavicon() {
    const links = document.querySelectorAll('link[rel~="icon"]');
    const href = links[links.length - 1]?.href;
    if (href && href !== lastIcon) {
      lastIcon = href;
      invoke("favicon_updated", { url: href }).catch(() => {});
    }
  }
  function watchFavicon() {
    if (watching || !document.documentElement) return;
    watching = true;
    reportFavicon();
    new MutationObserver(reportFavicon).observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["href"],
    });
  }
  watchFavicon();
  document.addEventListener("DOMContentLoaded", watchFavicon);

  // --- menu bar auto-hide: tap Alt to toggle (like Electron's autoHideMenuBar)
  let altAlone = false;
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
      if (ev.key === "Alt" && altAlone && MENU_AUTO_HIDE) {
        invoke("menu_toggle").catch(() => {});
      }
      altAlone = false;
    },
    true,
  );

  // --- Escape hides the window only when no chat is open -------------------
  if (ESC_TOGGLE) {
    addEventListener(
      "keydown",
      (ev) => {
        if (ev.key !== "Escape" || document.getElementById("main")) return;
        ev.preventDefault();
        ev.stopPropagation();
        invoke("window_toggle").catch(() => {});
      },
      true,
    );
  }
})();
"#;

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn js_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Builds the initialization script with the current configuration embedded.
pub fn script(host: &str, prefix: &str, esc_toggle: bool, menu_auto_hide: bool) -> String {
    SCRIPT
        .replace("__EW_HOST__", &js_string(host))
        .replace("__EW_PREFIX__", &js_string(prefix))
        .replace("__EW_ESC__", js_bool(esc_toggle))
        .replace("__EW_MENU__", js_bool(menu_auto_hide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_config_as_js_literals() {
        let script = script("web.whatsapp.com", "a\"b\\c", true, false);
        assert!(!script.contains("__EW_"));
        assert!(script.contains("const HOST = \"web.whatsapp.com\";"));
        assert!(script.contains("const PREFIX = \"a\\\"b\\\\c\";"));
        assert!(script.contains("const ESC_TOGGLE = true;"));
        assert!(script.contains("const MENU_AUTO_HIDE = false;"));
    }
}
