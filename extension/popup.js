async function send(type) {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) return;
  const response = await chrome.tabs.sendMessage(tab.id, { type });
  document.querySelector("#status").textContent = response?.ok ? "Filled." : (response?.error || "No matching entry.");
}

document.querySelector("#login").addEventListener("click", () => send("fill"));
document.querySelector("#otp").addEventListener("click", () => send("fill_otp"));
