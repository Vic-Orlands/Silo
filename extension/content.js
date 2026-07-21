async function requestLogin() {
  const response = await chrome.runtime.sendMessage({
    type: "get_login",
    url: window.location.href
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

async function requestOtp() {
  const response = await chrome.runtime.sendMessage({
    type: "get_otp",
    url: window.location.href
  });
  if (!response || !response.ok || !response.otp) return response || { ok: false, error: "No TOTP available." };
  const otp = document.querySelector('input[autocomplete="one-time-code"], input[name*="otp" i], input[name*="code" i], input[inputmode="numeric"]');
  if (!otp) return { ok: false, error: "No one-time-code field found." };
  otp.value = response.otp;
  otp.dispatchEvent(new Event("input", { bubbles: true }));
  otp.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true };
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === "fill") {
    requestLogin().then(sendResponse);
    return true;
  }
  if (message.type === "fill_otp") {
    requestOtp().then(sendResponse);
    return true;
  }
});
