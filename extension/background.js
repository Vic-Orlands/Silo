const HOST_NAME = "com.silo.native";
let nativePort = null;
let pending = [];

function getNativePort() {
  if (nativePort) return nativePort;
  nativePort = chrome.runtime.connectNative(HOST_NAME);
  nativePort.onMessage.addListener((response) => {
    const request = pending.shift();
    request?.resolve(response);
  });
  nativePort.onDisconnect.addListener(() => {
    const error = chrome.runtime.lastError?.message || "Silo native host disconnected.";
    for (const request of pending.splice(0)) request.reject(error);
    nativePort = null;
  });
  return nativePort;
}

function requestNative(message) {
  return new Promise((resolve, reject) => {
    pending.push({ resolve, reject });
    try {
      getNativePort().postMessage(message);
    } catch (error) {
      pending.pop();
      reject(error.message);
    }
  });
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type !== "get_login" && message.type !== "get_otp") return;
  requestNative(message)
    .then(sendResponse)
    .catch((error) => sendResponse({ ok: false, error: String(error) }));
  return true;
});
