async function requestLogin(entryId = null) {
  const response = await chrome.runtime.sendMessage({
    type: "get_login",
    url: window.location.href,
    entry_id: entryId
  });
  if (!response || !response.ok) return response || { ok: false, error: "No matching entry." };

  const username = document.querySelector('input[type="email"], input[name*="user" i], input[name*="login" i]');
  const password = document.querySelector('input[type="password"]');
  if (username && response.username) {
    username.value = response.username;
    username.dispatchEvent(new Event("input", { bubbles: true }));
    username.dispatchEvent(new Event("change", { bubbles: true }));
  }
  if (password && response.password) {
    password.value = response.password;
    password.dispatchEvent(new Event("input", { bubbles: true }));
    password.dispatchEvent(new Event("change", { bubbles: true }));
  }
  return { ok: true };
}

async function requestOtp(entryId = null) {
  const response = await chrome.runtime.sendMessage({
    type: "get_otp",
    url: window.location.href,
    entry_id: entryId
  });
  if (!response || !response.ok || !response.otp) return response || { ok: false, error: "No TOTP available." };
  const otp = document.querySelector('input[autocomplete="one-time-code"], input[name*="otp" i], input[name*="code" i], input[inputmode="numeric"]');
  if (!otp) return { ok: false, error: "No one-time-code field found." };
  otp.value = response.otp;
  otp.dispatchEvent(new Event("input", { bubbles: true }));
  otp.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true };
}

async function fillField(kind, entryId = null) {
  const response = await chrome.runtime.sendMessage({
    type: "get_login",
    url: window.location.href,
    entry_id: entryId
  });
  if (!response || !response.ok) return response || { ok: false, error: "No matching entry." };
  const selector = kind === "username"
    ? 'input[type="email"], input[name*="user" i], input[name*="login" i]'
    : 'input[type="password"]';
  const field = document.querySelector(selector);
  if (!field) return { ok: false, error: `No ${kind} field found.` };
  field.value = response[kind] || "";
  field.dispatchEvent(new Event("input", { bubbles: true }));
  field.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true };
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === "fill") {
    requestLogin(message.entryId || null).then(sendResponse);
    return true;
  }
  if (message.type === "fill_otp") {
    requestOtp(message.entryId || null).then(sendResponse);
    return true;
  }
  if (message.type === "fill_username" || message.type === "fill_password") {
    fillField(message.type === "fill_username" ? "username" : "password", message.entryId || null).then(sendResponse);
    return true;
  }
});

function watchLoginSubmission() {
  for (const form of document.forms) {
    form.addEventListener("submit", () => {
      const username = document.querySelector('input[type="email"], input[name*="user" i], input[name*="login" i]')?.value;
      const password = document.querySelector('input[type="password"]')?.value;
      if (username && password) {
        chrome.runtime.sendMessage({ type: "save_candidate", url: window.location.href, username, password });
      }
    }, { once: true });
  }
}

watchLoginSubmission();
