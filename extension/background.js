const HOST_NAME = "com.silo.native";
let nativePort = null;
let pending = [];
let saveCandidate = null;

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
  if (message.type === "save_candidate") {
    saveCandidate = {
      url: message.url,
      username: message.username,
      password: message.password,
    };
    sendResponse({ ok: true });
    return false;
  }
  if (message.type === "get_candidate") {
    sendResponse({ ok: true, candidate: saveCandidate });
    return false;
  }
  if (message.type === "dismiss_candidate") {
    saveCandidate = null;
    sendResponse({ ok: true });
    return false;
  }
  if (message.type === "save_login") {
    requestNative({
      type: "save_login",
      url: message.url,
      username: message.username,
      password: message.password,
    })
      .then((response) => {
        if (response?.ok) saveCandidate = null;
        sendResponse(response);
      })
      .catch((error) => sendResponse({ ok: false, error: String(error) }));
    return true;
  }
  if (message.type === "open_silo") {
    requestNative({ type: "open_silo" })
      .then(sendResponse)
      .catch((error) => sendResponse({ ok: false, error: String(error) }));
    return true;
  }
  if (message.type !== "get_login" && message.type !== "get_otp" && message.type !== "get_matches" && message.type !== "status") return;
  requestNative(message)
    .then(sendResponse)
    .catch((error) => sendResponse({ ok: false, error: String(error) }));
  return true;
});
