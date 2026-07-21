let selectedEntryId = null;

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
  const response = await chrome.tabs.sendMessage(tab.id, { type, entryId: selectedEntryId }).catch((error) => ({ ok: false, error: error.message }));
  setStatus(response?.ok ? "Filled." : (response?.error || "No matching entry."), response?.ok ? "success" : "error");
}

function renderMatches(matches) {
  const container = document.querySelector("#matches");
  container.replaceChildren();
  selectedEntryId = matches[0]?.id || null;
  for (const match of matches) {
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
  const response = await chrome.runtime.sendMessage({ type: "status" }).catch((error) => ({ ok: false, error: error.message }));
  if (!response?.ok) return setStatus(response?.error || "Silo broker unavailable.", "error");
  setStatus(
    response.unlocked ? "Silo is unlocked for this session." : "Silo is locked. Unlock it in the Silo app.",
    response.unlocked ? "success" : "",
  );
  if (response.unlocked) {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab?.url) {
      const matches = await chrome.runtime.sendMessage({ type: "get_matches", url: tab.url });
      renderMatches(matches?.matches || []);
    }
  }
}

document.querySelector("#refresh").addEventListener("click", checkSession);
document.querySelector("#login").addEventListener("click", () => sendToPage("fill"));
document.querySelector("#otp").addEventListener("click", () => sendToPage("fill_otp"));
showCurrentSite();
checkSession();
