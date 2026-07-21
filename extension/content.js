async function requestLogin() {
  const response = await chrome.runtime.sendMessage({
    type: "get_login",
    url: window.location.href
  });
  if (!response || !response.ok) return;

  const username = document.querySelector('input[type="email"], input[name*="user" i], input[name*="login" i]');
  const password = document.querySelector('input[type="password"]');
  if (username && response.username) username.value = response.username;
  if (password && response.password) password.value = response.password;
}

chrome.runtime.onMessage.addListener((message) => {
  if (message.type === "fill") requestLogin();
});

requestLogin();
