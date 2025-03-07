// high light the log-not-found paragraph
const logNotFound = document.getElementById('log-not-found');
if (location.hash === '#log-not-found') {
  logNotFound.classList.add('text-red-500');
}

// setup download links
const download = Array.from(document.getElementById('download').getElementsByTagName('div'));
download[0]/* windows */.addEventListener("click", function() {
  let url = "https://fs-im-kefu.7moor-fs1.com/ly/4d2c3f00-7d4c-11e5-af15-41bf63ae4ea0/1741366794351/log-reader.exe.zip";
  window.open(url);
})
download[1]/* macos */.addEventListener("click", function() {
  let url = "https://fs-im-kefu.7moor-fs1.com/ly/4d2c3f00-7d4c-11e5-af15-41bf63ae4ea0/1741366794172/Log Reader_0.1.0_universal.dmg.zip";
  window.open(url);
})