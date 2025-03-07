import React, { useEffect, useState } from "react"

/** a observer to test if ui enters the view */
export function useIntersectionObserver(param: {ref: React.MutableRefObject<HTMLElement>} & IntersectionObserverInit){
  const {ref, threshold = 0, rootMargin = "40px"} = param
  const [visible, setVisible] = useState(false)
  const [appeared, setAppeared] = useState(false)

  useEffect(() => {
    const observer = new IntersectionObserver(entry=> {
      setVisible(entry[0].isIntersecting)
      if (entry[0].isIntersecting){
        setAppeared(true)
      }
    }, { rootMargin, threshold })

    if (ref.current) observer.observe(ref.current)
    return () => {
      if (ref.current) observer.unobserve(ref.current)
    }
  }, [ref, threshold, rootMargin])

  return { visible, appeared }
}


export function useOS(): "windows" | "macos" {
  let ua = navigator.userAgent
  return ua.includes("Mac OS") ? "macos" : "windows"
}

export function useModKey(): "ctrl" | "meta" {
  let ua = navigator.userAgent
  return ua.includes("Mac OS") ? "meta" : "ctrl"
}

export function useSetting(key: string, defaultValue?: string): [string, (value: string)=> void] {
  const [value, setValue] = useState(localStorage.getItem(key) || defaultValue || "")
  const set = (value: string)=> {
    localStorage.setItem(key, value)
    setValue(value)
  }
  return [value, set]
}