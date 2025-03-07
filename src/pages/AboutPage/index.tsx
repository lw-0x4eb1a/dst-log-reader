import { invoke } from "@tauri-apps/api/core"
import { useCallback } from "react"

export default function AboutPage() {
  const open = useCallback((url: string)=> {
    invoke("open_url", {url})
  }, [])

  const linkClassName = "hover:text-blue-500 cursor-pointer underline"

  return (
    <div className="p-4 h-screen bg-slate-200">
      <h1 className="text-lg font-bold text-center">
        Don't Starve Log Reader
      </h1>
      <p className="text-center text-gray-400">
        version 0.1.0
      </p>
      <img
        width={100}
        draggable={false}
        src="https://i.loli.net/2021/09/29/7Z3J5w2v6q8Q1zg.png"
        alt="logo" 
        className="mx-auto mt-4"
      />
      <div className="mt-6 mb-4 h-px bg-gray-300"/>
      <div className="mx-auto text-center">
        <span 
          className={linkClassName} 
          onClick={()=> open("https://www.bing.com/")}>
          discussion
        </span>
        <span
          className={"ml-5 " + linkClassName}
          onClick={()=> open("https://www.github.com/luozhouyang/ds-log-reader")}>
          source code
        </span>
      </div>
      <p className="text-center mt-2">
        By:&nbsp;
        <span 
          className={linkClassName} 
          onClick={()=> open("https://space.bilibili.com/209631439")}>
          老王天天写Bug
        </span>
      </p>
    </div>
  )
}
