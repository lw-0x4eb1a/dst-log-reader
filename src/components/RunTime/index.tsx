import { i18n } from "@lingui/core"
import { defineMessage } from "@lingui/core/macro"

function norm(value: number): string {
  return value < 10 ? `0${value}` : `${value}`
}

export function formatRunTime(time: number[] | [number, number, number]) {
  const [hour, minute, second] = time
  if (hour === 0 && minute === 0 && second === 0)
    return "-"
  else if (hour === 0 && minute === 0)
    return i18n._( second === 1 ? defineMessage`${second} sec` : defineMessage`${second} secs` )
  else if (hour === 0)
    return `${norm(minute)}:${norm(second)}`
  else 
    return `${hour}:${norm(minute)}:${norm(second)}`
}
