/* fullme 页签栏前端：无打包器，依赖 withGlobalTauri 注入的全局 API */
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const tabsEl = document.getElementById("tabs");
const bodyEl = document.body;

/** 渲染页签快照 */
function render(snap) {
  bodyEl.classList.toggle("no-tabs", snap.tabs.length === 0);
  tabsEl.textContent = "";

  for (const tab of snap.tabs) {
    const el = document.createElement("div");
    el.className = "tab" + (tab.id === snap.active ? " active" : "");
    el.dataset.id = String(tab.id);
    el.title = tab.url;

    const title = document.createElement("span");
    title.className = "tab-title";
    title.textContent = tab.title || tab.url;
    el.appendChild(title);

    const close = document.createElement("span");
    close.className = "tab-close";
    close.dataset.close = String(tab.id);
    close.textContent = "×";
    close.title = "关闭页签";
    el.appendChild(close);

    tabsEl.appendChild(el);
  }
}

/** 点击切换页签 / 点 × 关闭页签 */
tabsEl.addEventListener("click", (e) => {
  const closeEl = e.target.closest("[data-close]");
  if (closeEl) {
    invoke("close_tab", { id: Number(closeEl.dataset.close) }).catch(showError);
    return;
  }
  const tabEl = e.target.closest(".tab");
  if (tabEl) {
    invoke("switch_tab", { id: Number(tabEl.dataset.id) }).catch(showError);
  }
});

/** 右键关闭页签 */
tabsEl.addEventListener("contextmenu", (e) => {
  const tabEl = e.target.closest(".tab");
  if (tabEl) {
    e.preventDefault();
    invoke("close_tab", { id: Number(tabEl.dataset.id) }).catch(showError);
  }
});

document
  .getElementById("btn-reload")
  .addEventListener("click", () => invoke("reload_active").catch(showError));

const modeBtn = document.getElementById("btn-mode");

/** 按窗口模式切换按钮图标：正常 → `─`（缩小），缩小 → `⤢`（恢复） */
function renderMode(snap) {
  modeBtn.textContent = snap.compact ? "⤢" : "─";
  modeBtn.title = snap.compact ? "恢复正常大小" : "缩小窗口（仅保留标题栏）";
}

modeBtn.addEventListener("click", () =>
  invoke("toggle_window_mode").catch(showError)
);

document
  .getElementById("btn-quit")
  .addEventListener("click", () => invoke("quit_app").catch(showError));

function showError(err) {
  console.error("fullme:", err);
}

// 先注册监听，再拉取快照，避免错过事件
listen("tabs-updated", (e) => render(e.payload)).catch(showError);
listen("window-mode-updated", (e) => renderMode(e.payload)).catch(showError);
invoke("get_tabs")
  .then(render)
  .catch(showError);
