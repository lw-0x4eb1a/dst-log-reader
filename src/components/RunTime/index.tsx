function norm(value: number): string {
  return value < 10 ? `0${value}` : `${value}`
}

export function formatRunTime(time: number[] | [number, number, number]) {
  const [hour, minute, second] = time
  if (hour === 0 && minute === 0 && second === 0)
    return "-"
  else if (hour === 0 && minute === 0)
    return `${second} secs`
  else if (hour === 0)
    return `${norm(minute)}:${norm(second)}`
  else 
    return `${hour}:${norm(minute)}:${norm(second)}`
}
