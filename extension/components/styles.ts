// CSS injected into the Shadow DOM. Kept as a string so it's trivially
// portable and doesn't depend on a bundler-side loader.

export const PANEL_STYLES = `
:host, :root { all: initial; }
* { box-sizing: border-box; font-family: system-ui, -apple-system, sans-serif; }
.libre-cr-tour {
  position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%);
  width: min(720px, calc(100vw - 48px)); background: #fff; color: #1f2328;
  border: 1px solid #d0d7de; border-radius: 10px; box-shadow: 0 12px 32px rgba(140, 149, 159, 0.3);
  z-index: 2147483646; font-family: system-ui, -apple-system, sans-serif;
}
.libre-cr-tour-nav { display: flex; align-items: center; gap: 8px; padding: 10px 12px; border-bottom: 1px solid #d8dee4; }
.libre-cr-tour-btn {
  font: 600 14px/1 system-ui, -apple-system, sans-serif; padding: 10px 14px; border-radius: 8px;
  border: 1px solid #d0d7de; background: #f6f8fa; color: #1f2328; cursor: pointer;
}
.libre-cr-tour-btn:hover:not(:disabled) { background: #eaeef2; }
.libre-cr-tour-btn:disabled { opacity: 0.45; cursor: default; }
.libre-cr-tour-btn.primary { background: #1f883d; border-color: #1f883d; color: #fff; }
.libre-cr-tour-btn.primary:hover:not(:disabled) { background: #1a7f37; }
.libre-cr-tour-count { font: 600 14px/1 system-ui, -apple-system, sans-serif; min-width: 64px; text-align: center; }
.libre-cr-tour-btn:last-child { margin-left: auto; }
.libre-cr-tour-body { padding: 12px 14px 14px; }
.libre-cr-tour-title { font: 600 15px/1.3 system-ui, -apple-system, sans-serif; }
.libre-cr-tour-where { font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; color: #57606a; margin-top: 2px; }
.libre-cr-tour-detail { font: 14px/1.5 system-ui, -apple-system, sans-serif; margin-top: 8px; white-space: pre-wrap; }
.libre-cr-reopen {
  position: fixed;
  right: 24px;
  bottom: 24px;
  width: 44px;
  height: 44px;
  border-radius: 50%;
  border: 1px solid #d0d7de;
  background: #fff;
  color: #1f2328;
  font: 600 13px/1 system-ui, -apple-system, sans-serif;
  box-shadow: 0 8px 24px rgba(140, 149, 159, 0.2);
  cursor: pointer;
  z-index: 2147483646;
}
.libre-cr-reopen:hover { background: #f6f8fa; }
.libre-cr-shell {
  position: fixed;
  top: 80px;
  right: 24px;
  width: 380px;
  min-width: 280px;
  max-width: 90vw;
  background: #fff;
  color: #1f2328;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  box-shadow: 0 8px 24px rgba(140, 149, 159, 0.2);
  z-index: 2147483646;
  display: flex;
  flex-direction: column;
  font-size: 13px;
  max-height: 80vh;
  overflow: hidden;
}
.libre-cr-titlebar {
  background: #f6f8fa;
  padding: 8px 12px;
  border-bottom: 1px solid #d0d7de;
  cursor: move;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}
.libre-cr-titlebar button {
  background: transparent;
  border: 0;
  cursor: pointer;
  color: #57606a;
  font-size: 14px;
  padding: 2px 6px;
}
.libre-cr-selection {
  background: #ddf4ff;
  padding: 6px 10px;
  font-size: 12px;
  border-bottom: 1px solid #cfe6f6;
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.libre-cr-conversation {
  padding: 8px 10px;
  flex: 1 1 auto;
  overflow-y: auto;
  min-height: 80px;
}
.libre-cr-turn {
  border: 1px solid #d0d7de;
  border-radius: 4px;
  padding: 6px 8px;
  margin-bottom: 6px;
  background: #fff;
}
.libre-cr-turn.note { background: #f6f8fa; }
.libre-cr-turn .q { font-weight: 600; margin-bottom: 2px; }
.libre-cr-turn .a { white-space: pre-wrap; }
.libre-cr-thinking { font-size: 12px; color: #57606a; margin-top: 4px; }
.libre-cr-thinking summary { cursor: pointer; }
.libre-cr-verbs {
  padding: 6px 10px;
  border-top: 1px solid #d0d7de;
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.libre-cr-verbs button {
  background: #f6f8fa;
  border: 1px solid #d0d7de;
  padding: 3px 8px;
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
}
.libre-cr-verbs button:disabled { opacity: 0.4; cursor: not-allowed; }
.libre-cr-input {
  border-top: 1px solid #d0d7de;
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.libre-cr-input textarea {
  width: 100%;
  min-height: 50px;
  resize: vertical;
  font: inherit;
  border: 1px solid #d0d7de;
  border-radius: 4px;
  padding: 4px 6px;
}
.libre-cr-input-actions { display: flex; justify-content: flex-end; gap: 6px; }
.libre-cr-input-actions button {
  border: 1px solid #d0d7de;
  background: #f6f8fa;
  padding: 3px 10px;
  border-radius: 4px;
  cursor: pointer;
}
.libre-cr-input-actions button.primary {
  background: #1f883d; color: white; border-color: #1f883d;
}
.libre-cr-footer {
  padding: 6px 10px;
  border-top: 1px solid #d0d7de;
  font-size: 12px;
  color: #57606a;
  display: flex;
  justify-content: space-between;
}
.libre-cr-footer button {
  background: transparent;
  border: 0;
  cursor: pointer;
  color: #0969da;
  font-size: 12px;
}
.libre-cr-banner {
  background: #fff8c5;
  border: 1px solid #d4a72c;
  padding: 6px 10px;
  font-size: 12px;
  color: #4d3800;
  margin: 6px;
  border-radius: 4px;
}
.libre-cr-error {
  background: #ffebe9;
  border: 1px solid #ff8182;
  padding: 6px 10px;
  font-size: 12px;
  color: #82071e;
  margin: 6px;
  border-radius: 4px;
}
`;
