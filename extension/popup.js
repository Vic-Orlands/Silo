async function sendToPage(type) {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) return;
  const response = await chrome.tabs.sendMessage(tab.id, { type }).catch((error) => ({ ok: false, error: error.message }));
  document.querySelector("#status").textContent = response?.ok ? "Filled." : (response?.error || "No matching entry.");
}

document.querySelector("#unlock").addEventListener("click", async () => {
  const input = document.querySelector("#password");
  const response = await chrome.runtime.sendMessage({ type: "unlock", password: input.value });
  input.value = "";
  document.querySelector("#status").textContent = response?.ok ? "Unlocked for this session." : (response?.error || "Could not unlock.");
});
document.querySelector("#login").addEventListener("click", () => sendToPage("fill"));
document.querySelector("#otp").addEventListener("click", () => sendToPage("fill_otp"));
