import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLingui } from '@lingui/react/macro'

const glommerFrames = import.meta.glob("../../assets/glommer/*png", {"as": "url"})
const MAX_FRAME = 48

export default function index() {
  const [url, setUrl] = useState("")
  const [hover, setHover] = useState(false)
  const [frame, setFrame] = useState(1)
  const {i18n} = useLingui()
  const locale = i18n.locale

  useEffect(()=> {
    glommerFrames[`../../assets/glommer/${frame}.png`]().then((v)=> {
      setUrl(v)
    })
  }, [frame])

  useEffect(()=> {
    if (hover) {
      const timer = setInterval(()=> {
        setFrame((prev)=> prev === MAX_FRAME ? 1 : prev + 1)
      }, 1000/30)
      return ()=> clearInterval(timer)
    }
  }, [hover])

  return (
    <img
      width={40}
      onMouseEnter={()=> setHover(true)}
      onMouseLeave={()=> [setHover(false), setFrame(1)]}
      onClick={()=> invoke("open_tool_menu", {locale})}
      draggable={false}
      className="user-select-none transition-all"
      style={{
        transform: hover ? "scale(1.25)" : "",
        transformOrigin: "50% 60%",
        filter: "drop-shadow(1px 2px 1px #0009)"
      }}
      src={url} alt="glommer"/>
  )
}
