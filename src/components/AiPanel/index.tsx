import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { WebviewWindow } from "@tauri-apps/api/webviewWindow"
import { useCallback, useEffect, useRef, useState } from "react"
import { useLingui } from "@lingui/react/macro"
import {
  AI_EVENT_NAME,
  configProblem,
  effectiveModel,
  formatTokens,
  loadAiConfig,
  resolveMod,
  type AiCause,
  type AiConfig,
  type AiMod,
  type AiResult,
  type AiState,
  type AiUsage,
} from "../../ai"

const buttonClass = "px-2 py-1 text-sm border rounded-sm cursor-pointer transition-all \
  bg-white/90 border-slate-300 hover:bg-slate-100"
const primaryClass = "px-3 py-1 text-sm rounded-sm cursor-pointer transition-all \
  bg-blue-500 text-white border border-blue-500 hover:bg-blue-600 \
  disabled:opacity-50 disabled:cursor-not-allowed"

type AiPanelProps = {
  open: boolean,
  onClose: ()=> void,
}

export default function AiPanel(props: AiPanelProps) {
  const { open, onClose } = props
  const { t } = useLingui()
  const label = WebviewWindow.getCurrent().label
  const [view, setView] = useState<AiState>({state: "idle"})
  const [now, setNow] = useState(()=> Date.now())
  const requestRef = useRef("")
  const startedRef = useRef(false)
  const tailRef = useRef<HTMLPreElement>(null)

  const start = useCallback((cfg: AiConfig)=> {
    const requestId = `${Date.now()}-${Math.random().toString(16).slice(2)}`
    requestRef.current = requestId
    setView({state: "running", status: "", tail: "", startedAt: Date.now()})
    invoke("ai_analyze", {
      label,
      requestId,
      config: {
        provider: cfg.provider,
        apiKey: cfg.apiKey.trim(),
        model: effectiveModel(cfg),
        baseUrl: cfg.baseUrl.trim(),
        gameDir: cfg.gameDir.trim(),
        lang: window.currentLocale || "zh",
      },
    }).catch(err=> {
      if (requestRef.current === requestId) {
        setView({state: "error", message: String(err)})
      }
    })
  }, [label])

  // progress reported by the rust side
  useEffect(()=> {
    const unlisten = listen<string>(AI_EVENT_NAME, event=> {
      let data: any
      try {
        data = JSON.parse(event.payload)
      }
      catch {
        return
      }
      if (!requestRef.current || data.id !== requestRef.current) return
      switch (data.state) {
        case "progress":
          setView(v=> v.state === "running" && data.text ? {...v, status: data.text} : v)
          break
        case "thinking":
          setView(v=> v.state === "running" && !v.tail ? {...v, status: t`模型正在思考…`} : v)
          break
        case "chunk":
          setView(v=> v.state === "running" ? {...v, tail: data.text || v.tail} : v)
          break
        case "usage":
          setView(v=> (v.state === "running" || v.state === "done")
            ? {...v, usage: data.usage as AiUsage}
            : v)
          break
        case "done":
          setView({
            state: "done",
            result: normalizeResult(data.result),
            usage: data.usage as AiUsage | undefined,
          })
          break
        case "error":
          setView({state: "error", message: data.message || ""})
          break
      }
    })
    return ()=> { unlisten.then(f=> f()) }
  }, [])

  // first open starts the analysis
  useEffect(()=> {
    if (!open || startedRef.current) return
    startedRef.current = true
    const cfg = loadAiConfig()
    if (configProblem(cfg)) {
      setView({state: "error", message: "", needsConfig: true})
    }
    else {
      start(cfg)
    }
  }, [open, start])

  // live elapsed time
  useEffect(()=> {
    if (view.state !== "running") return
    const timer = setInterval(()=> setNow(Date.now()), 500)
    return ()=> clearInterval(timer)
  }, [view.state])

  // keep the streamed tail scrolled to the bottom
  const tail = view.state === "running" ? view.tail : ""
  useEffect(()=> {
    const element = tailRef.current
    if (element) element.scrollTop = element.scrollHeight
  }, [tail])

  const rerun = useCallback(()=> {
    const cfg = loadAiConfig()
    if (configProblem(cfg)) {
      setView({state: "error", message: "", needsConfig: true})
      return
    }
    start(cfg)
  }, [start])

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 flex justify-end"
      style={{backgroundColor: "#00000030"}}
      onClick={onClose}>
      <div
        className="h-full flex flex-col bg-blue-50 shadow-2xl border-l border-slate-300"
        style={{width: 420, maxWidth: "85vw"}}
        onClick={e=> e.stopPropagation()}>
        <div className="flex items-center justify-between px-3 py-2 border-b border-slate-300 bg-blue-100/60">
          <h1 className="font-bold text-gray-700">{t`AI 分析`}</h1>
          <button
            className="px-2 text-lg leading-none text-gray-500 rounded-sm cursor-pointer hover:bg-black/10"
            onClick={onClose}>×</button>
        </div>
        <div className="flex-1 overflow-auto">
          {
            view.state === "running" &&
            <RunningView
              status={view.status}
              tail={view.tail}
              usage={view.usage}
              startedAt={view.startedAt}
              now={now}
              tailRef={tailRef}/>
          }
          {
            view.state === "done" &&
            <ResultView result={view.result} usage={view.usage} onRerun={rerun}/>
          }
          {
            view.state === "error" &&
            <ErrorView
              message={view.message}
              needsConfig={!!view.needsConfig}
              onRetry={rerun}/>
          }
          {
            view.state === "idle" &&
            <div className="p-4 text-sm text-gray-500">{t`正在准备…`}</div>
          }
        </div>
        <div className="px-3 py-2 border-t border-slate-300 text-center text-xs text-gray-400">
          {t`AI 分析，仅供参考`}
        </div>
      </div>
    </div>
  )
}

function normalizeResult(raw: any): AiResult {
  const result = (raw || {}) as AiResult
  return {
    summary: result.summary || "",
    causes: result.causes || [],
    mods: result.mods || [],
    suggestions: result.suggestions || [],
    raw: result.raw,
  }
}

function UsageLine(props: {usage?: AiUsage, className?: string}) {
  const { usage } = props
  const { t } = useLingui()
  if (!usage || !usage.prompt_tokens) return null
  // share of the input tokens that came from the provider cache
  const cacheRate = ((usage.cached_tokens / usage.prompt_tokens) * 100).toFixed(1)
  const parts = [
    `${t`Tokens`} ${formatTokens(usage.prompt_tokens + usage.completion_tokens)}`,
    `${t`输入`} ${formatTokens(usage.prompt_tokens)}`,
    `${t`输出`} ${formatTokens(usage.completion_tokens)}`,
    `${t`缓存命中`} ${cacheRate}%`,
  ]
  const context = usage.context_limit
    ? `${t`上下文`} ${formatTokens(usage.context_tokens)} / ${formatTokens(usage.context_limit)}`
      + ` (${((usage.context_tokens / usage.context_limit) * 100).toFixed(1)}%)`
    : `${t`上下文`} ${formatTokens(usage.context_tokens)}`
  // exact numbers on hover, the line itself stays compact
  const detail = [
    `${t`输入`} ${usage.prompt_tokens}`,
    `${t`输出`} ${usage.completion_tokens}`,
    `${t`缓存命中`} ${usage.cached_tokens} (${cacheRate}%)`,
    `${t`上下文`} ${usage.context_tokens}`,
  ].join(" · ")
  return (
    <div
      title={detail}
      className={["text-[11px] leading-5 text-gray-400", props.className || ""].join(" ")}>
      <div>{parts.join(" · ")}</div>
      <div>{context}</div>
    </div>
  )
}

function RunningView(props: {
  status: string,
  tail: string,
  usage?: AiUsage,
  startedAt: number,
  now: number,
  tailRef: React.RefObject<HTMLPreElement>,
}) {
  const { t } = useLingui()
  const { status, tail, usage, startedAt, now, tailRef } = props
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000))
  return (
    <div className="flex h-full flex-col p-3">
      <div className="mb-2 flex items-center gap-2 text-sm text-gray-600">
        <div className="h-4 w-4 shrink-0 animate-spin rounded-full border-2 border-blue-400 border-t-transparent"/>
        <span className="flex-1 truncate">{status || t`正在分析…`}</span>
        <span className="shrink-0 text-xs text-gray-400">{seconds}s</span>
      </div>
      <UsageLine usage={usage} className="mb-2"/>
      {
        tail
          ? <pre
            ref={tailRef}
            className="flex-1 overflow-auto whitespace-pre-wrap break-words rounded-sm border border-slate-200 bg-white/70 p-2 text-[11px] leading-4 text-gray-500">
            {tail}
          </pre>
          : <div className="flex-1 rounded-sm border border-dashed border-slate-200 p-2 text-xs text-gray-400">
            {t`AI 正在阅读日志、游戏与模组代码…`}
          </div>
      }
    </div>
  )
}

function ErrorView(props: {message: string, needsConfig: boolean, onRetry: ()=> void}) {
  const { t } = useLingui()
  const { message, needsConfig, onRetry } = props
  return (
    <div className="p-3">
      <div className="mb-3 select-text whitespace-pre-wrap break-words rounded-sm border border-red-300 bg-red-100/70 p-2 text-sm text-red-600">
        {needsConfig ? t`请先在左侧配置 API Key 与模型名称。` : (message || t`分析失败`)}
      </div>
      <button className={primaryClass} onClick={onRetry}>{t`重试`}</button>
    </div>
  )
}

function ResultView(props: {result: AiResult, usage?: AiUsage, onRerun: ()=> void}) {
  const { t } = useLingui()
  const { result, usage, onRerun } = props
  const causes: AiCause[] = result.causes || []
  const mods: AiMod[] = result.mods || []
  const suggestions = result.suggestions || []
  const empty = !result.summary && !causes.length && !mods.length && !suggestions.length

  if (empty) {
    return (
      <div className="p-3 text-sm">
        <div className="mb-3 rounded-sm border border-amber-300 bg-amber-50 p-2 text-amber-700">
          {t`模型这次没有返回可展示的结论，请重试；若反复出现，可换用其它模型。`}
        </div>
        <button className={buttonClass} onClick={onRerun}>{t`重新分析`}</button>
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex-1 select-text p-3 text-sm text-gray-700">
        {
          result.summary &&
          <p className="mb-3 whitespace-pre-wrap rounded-sm border border-slate-300 bg-white/90 p-2">
            {result.summary}
          </p>
        }
        {
          causes.length > 0 &&
          <>
            <h3 className="mb-1 font-bold text-gray-600">{t`可能的原因`}</h3>
            {
              causes.map((cause, i)=> 
                <div key={i} className="mb-2 rounded-sm border border-slate-300 bg-white/90 p-2">
                  <p className="font-bold">{i + 1}. {cause.title}</p>
                  {
                    cause.detail &&
                    <p className="mt-1 whitespace-pre-wrap text-gray-600">{cause.detail}</p>
                  }
                  {
                    cause.mods && cause.mods.length > 0 &&
                    <div className="mt-1 flex flex-wrap gap-1">
                      {cause.mods.map((mod, j)=> <ModChip key={j} mod={mod}/>)}
                    </div>
                  }
                </div>
              )
            }
          </>
        }
        {
          mods.length > 0 &&
          <>
            <h3 className="mt-3 mb-1 font-bold text-gray-600">{t`相关模组`}</h3>
            {
              mods.map((mod, i)=> 
                <div key={i} className="mb-2 rounded-sm border border-slate-300 bg-white/90 p-2">
                  <ModTitle mod={mod}/>
                  {
                    mod.reason &&
                    <p className="mt-1 whitespace-pre-wrap text-gray-600">{mod.reason}</p>
                  }
                </div>
              )
            }
          </>
        }
        {
          suggestions.length > 0 &&
          <>
            <h3 className="mt-3 mb-1 font-bold text-gray-600">{t`处理建议`}</h3>
            <ul className="list-disc space-y-1 pl-5 text-gray-600">
              {suggestions.map((text, i)=> <li key={i}>{text}</li>)}
            </ul>
          </>
        }
        <button className={buttonClass + " mt-4"} onClick={onRerun}>{t`重新分析`}</button>
      </div>
      <div className="border-t border-slate-200 px-3 py-1.5">
        <UsageLine usage={usage}/>
      </div>
    </div>
  )
}

function ModChip(props: {mod: string}) {
  const info = resolveMod(props.mod)
  if (!info.label) return null
  if (info.workshopId) {
    return (
      <span
        onClick={()=> window.visitMod(info.workshopId)}
        title={info.workshopId}
        className="cursor-pointer rounded-sm border border-blue-200 bg-blue-100 px-1.5 py-0.5 text-xs text-blue-700 hover:bg-blue-200">
        {info.label}
      </span>
    )
  }
  return (
    <span className="rounded-sm border border-slate-200 bg-slate-100 px-1.5 py-0.5 text-xs text-gray-600">
      {info.label}
    </span>
  )
}

function ModTitle(props: {mod: AiMod}) {
  const { mod } = props
  const info = resolveMod(mod.moddir || mod.name)
  return (
    <p className="font-bold">
      {info.label || mod.name}
      {
        info.workshopId &&
        <span
          onClick={()=> window.visitMod(info.workshopId)}
          className="ml-2 cursor-pointer text-xs font-normal text-blue-500 underline hover:text-blue-400">
          {info.workshopId}
        </span>
      }
    </p>
  )
}
