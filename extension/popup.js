let selectedEntryId = null;
let allMatches = [];
let pendingAction = null;
let candidate = null;

async function showCurrentSite() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.url) return;
  try {
    document.querySelector("#site").textContent = new URL(tab.url).hostname || "Current page";
  } catch (_) {
    document.querySelector("#site").textContent = "Current page";
  }
}

async function sendToPage(type) {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) return setStatus("No active page.", "error");
  const response = await chrome.tabs.sendMessage(tab.id, { type, entryId: selectedEntryId })
    .catch((error) => ({ ok: false, error: error.message }));
  setStatus(response?.ok ? "Filled." : (response?.error || "No matching entry."), response?.ok ? "success" : "error");
}

function requestApproval(type, label) {
  pendingAction = type;
  document.querySelector("#approval-copy").textContent = `Approve ${label}?`;
  document.querySelector("#approval").hidden = false;
  document.querySelector(".actions").hidden = true;
}

function cancelApproval() {
  pendingAction = null;
  document.querySelector("#approval").hidden = true;
  document.querySelector(".actions").hidden = false;
}

async function approveAction() {
  const action = pendingAction;
  cancelApproval();
  if (action) await sendToPage(action);
}

function renderMatches(matches) {
  const container = document.querySelector("#matches");
  container.replaceChildren();
  const query = document.querySelector("#search").value.trim().toLowerCase();
  const filtered = matches.filter((match) => `${match.name} ${match.username}`.toLowerCase().includes(query));
  selectedEntryId = filtered[0]?.id || null;
  for (const match of filtered) {
    const button = document.createElement("button");
    button.className = `match${match.id === selectedEntryId ? " selected" : ""}`;
    button.innerHTML = `<span>${escapeHtml(match.name)}</span><small>${escapeHtml(match.username)}</small>`;
    button.addEventListener("click", () => {
      selectedEntryId = match.id;
      renderMatches(matches);
      setStatus(`${match.name} selected.`, "success");
    });
    container.append(button);
  }
}

function escapeHtml(value) {
  const element = document.createElement("span");
  element.textContent = value;
  return element.innerHTML;
}

function setStatus(message, kind = "") {
  const status = document.querySelector("#status");
  status.textContent = message;
  status.className = kind;
}

async function checkSession() {
  const response = await chrome.runtime.sendMessage({ type: "status" })
    .catch((error) => ({ ok: false, error: error.message }));
  if (!response?.ok) return setStatus(response?.error || "Silo broker unavailable.", "error");
  setStatus(
    response.unlocked ? "Silo is unlocked for this session." : "Silo is locked. Unlock it in the Silo app.",
    response.unlocked ? "success" : "",
  );
  if (response.unlocked) {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab?.url) {
      const matches = await chrome.runtime.sendMessage({ type: "get_matches", url: tab.url });
      allMatches = matches?.matches || [];
      renderMatches(allMatches);
    }
  }
  const pending = await chrome.runtime.sendMessage({ type: "get_candidate" }).catch(() => null);
  candidate = pending?.candidate || null;
  document.querySelector("#candidate").hidden = !candidate;
  if (candidate) {
    document.querySelector("#candidate-copy").textContent = `Save ${candidate.username} for ${new URL(candidate.url).hostname}?`;
  }
}

document.querySelector("#refresh").addEventListener("click", checkSession);
document.querySelector("#search").addEventListener("input", () => renderMatches(allMatches));
document.querySelector("#login").addEventListener("click", () => requestApproval("fill", "username and password"));
document.querySelector("#username").addEventListener("click", () => requestApproval("fill_username", "the username"));
document.querySelector("#password").addEventListener("click", () => requestApproval("fill_password", "the password"));
document.querySelector("#otp").addEventListener("click", () => requestApproval("fill_otp", "the one-time code"));
document.querySelector("#approve").addEventListener("click", approveAction);
document.querySelector("#cancel-approval").addEventListener("click", cancelApproval);
document.querySelector("#dismiss-candidate").addEventListener("click", () => {
  candidate = null;
  document.querySelector("#candidate").hidden = true;
  chrome.runtime.sendMessage({ type: "dismiss_candidate" });
});
document.querySelector("#save-login").addEventListener("click", async () => {
  if (!candidate) return;
  const response = await chrome.runtime.sendMessage({ type: "save_login", ...candidate });
  setStatus(response?.ok ? "Login saved." : (response?.error || "Could not save login."), response?.ok ? "success" : "error");
  if (response?.ok) {
    candidate = null;
    document.querySelector("#candidate").hidden = true;
  }
});
document.querySelector("#open-silo").addEventListener("click", () => {
  chrome.runtime.sendMessage({ type: "open_silo" })
    .then((response) => setStatus(response?.ok ? "Silo is opening." : (response?.error || "Could not open Silo."), response?.ok ? "success" : "error"));
});
showCurrentSite();
checkSession();
