const HOST_NAME = "com.silo.native";

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type !== "get_login" && message.type !== "get_otp") return;

  const port = chrome.runtime.connectNative(HOST_NAME);
  let responded = false;
  port.onMessage.addListener((response) => {
    responded = true;
    sendResponse(response);
    port.disconnect();
  });
  port.onDisconnect.addListener(() => {
    if (!responded && chrome.runtime.lastError) {
      sendResponse({ ok: false, error: chrome.runtime.lastError.message });
    }
  });
  port.postMessage(message);
  return true;
});
