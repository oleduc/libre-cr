// CSS injected into the Shadow DOM. Kept as a string so it's trivially
// portable and doesn't depend on a bundler-side loader.

export const PANEL_STYLES = `
:host, :root { all: initial; }
* { box-sizing: border-box; font-family: system-ui, -apple-system, sans-serif; }
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
