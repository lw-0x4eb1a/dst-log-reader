import { useEffect, useReducer } from "react"
import { i18n } from "@lingui/core"
import { defineMessage, plural } from "@lingui/core/macro"

type ModifiedTimeProps = {
  mtime: number,
  className?: string,
  autoUpdate?: boolean,
}

function formatModifiedTime(mtime: number) {
  if (mtime <= 0) return "-"
  const current = Date.now() / 1000
  const dt = current - mtime
  if (dt <= 0) return "-"
  if (dt <= 60) return i18n._(defineMessage({message: "just now"}))
  if (dt <= 60*60) return i18n._(plural(Math.floor(dt/60), {one: "# minute ago", other: "# minutes ago"}))
  if (dt <= 24*60*60) return i18n._(plural(Math.floor(dt/60/60), {one: "# hour ago", other: "# hours ago"}))
  if (dt <= 48*60*60) return i18n._(defineMessage({message: "yesterday"}))
  return new Date(mtime * 1000).toLocaleString()
}

export default function ModifiedTime(props: ModifiedTimeProps) {
  const {mtime, autoUpdate, className} = props
  const [_, forceUpdate] = useReducer((x)=> x+1, 0)

  useEffect(()=> {
    if (autoUpdate !== false) {
      const interval = setInterval(forceUpdate, 10*1000)
      return ()=> clearInterval(interval)
    }
  }, [autoUpdate])

  return (
    <span className={className}>
      {
        formatModifiedTime(mtime)
      }
    </span>
  )
}
