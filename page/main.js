// high light the log-not-found paragraph
const logNotFound = document.getElementById('log-not-found');
if (location.hash === '#log-not-found') {
  logNotFound.style.color = 'red';
}

// setup download links
const download = Array.from(document.getElementById('download').getElementsByTagName('div'));
download[0]/* windows */.addEventListener("click", function() {
  let url = "https://github.com/lw-0x4eb1a/dst-log-reader/releases/download/v0.1.0-build.2/Log.Reader_0.1.0_x64-setup.exe";
  window.open(url);
})
download[1]/* macos */.addEventListener("click", function() {
  let url = "https://github.com/lw-0x4eb1a/dst-log-reader/releases/download/v0.1.0-build.2/Log.Reader_0.1.0_universal.dmg";
  window.open(url);
})