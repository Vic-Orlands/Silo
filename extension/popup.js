async function sendToPage(type) {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) return setStatus("No active page.", "error");
  const response = await chrome.tabs.sendMessage(tab.id, { type }).catch((error) => ({ ok: false, error: error.message }));
  setStatus(response?.ok ? "Filled." : (response?.error || "No matching entry."), response?.ok ? "success" : "error");
}

function setStatus(message, kind = "") {
  const status = document.querySelector("#status");
  status.textContent = message;
  status.className = kind;
}

document.querySelector("#unlock").addEventListener("click", async () => {
  const input = document.querySelector("#password");
  const button = document.querySelector("#unlock");
  button.disabled = true;
  const response = await chrome.runtime.sendMessage({ type: "unlock", password: input.value }).catch((error) => ({ ok: false, error: error.message }));
  input.value = "";
  button.disabled = false;
  setStatus(response?.ok ? "Unlocked for this session." : (response?.error || "Could not unlock."), response?.ok ? "success" : "error");
});
document.querySelector("#login").addEventListener("click", () => sendToPage("fill"));
document.querySelector("#otp").addEventListener("click", () => sendToPage("fill_otp"));
