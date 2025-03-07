// high light the log-not-found paragraph
const logNotFound = document.getElementById('log-not-found');
if (location.hash === '#log-not-found') {
  logNotFound.classList.add('text-red-500');
}

// setup download links
const download = Array.from(document.getElementById('download').getElementsByTagName('div'));
download[0]/* windows */.addEventListener("click", function() {
  let url = "https://www.baidu.com";
  window.open(url);
})
download[1]/* macos */.addEventListener("click", function() {
  let url = "https://www.bing.com";
  window.open(url);
})