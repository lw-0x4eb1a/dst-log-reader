import { invoke } from "@tauri-apps/api/core"

const ICON_CACHE = new Map<string, string>()
const ICON_FAILED_CACHE = new Map<string, number>()

function refresh() {
  // console.log("refresh...")
  for (let img of document.querySelectorAll("img")) {
    let id = img.getAttribute("data-id")
    if (!id) continue
    let url  = getIconPath(id)
    if (url) {
      img.setAttribute("src", url)
    }
  }
}

setInterval(refresh, 200)

export function getIconPath(id: string): string {
  if (ICON_CACHE.has(id)) {
    return ICON_CACHE.get(id)
  }
  if (ICON_FAILED_CACHE.get(id) > 5) {
    return ""
  }

  if (/[0-9]{5,15}/.test(id)) {
    invoke<string>("get_steam_workshop_icon", {id}).then(
      res=> {
        if (res) {
          console.log(id, res)
          ICON_CACHE.set(id, res)
          refresh()
        }
      },
      err=> {
        console.warn("Failed to get icon", err)
        ICON_FAILED_CACHE.set(id, ICON_FAILED_CACHE.get(id) + 1 || 1)
      }
    )
  }
  else {
    console.log("Not a valid workshop id", id)
    return ""
  }
}