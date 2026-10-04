import { invoke } from '@tauri-apps/api/core'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import React, { useState } from 'react'
import { useLingui } from '@lingui/react/macro'
import type { LogComment, NavigateAction } from '../../pages/LogBrowserPage'
import { formatRunTime } from '../RunTime'
import AiPanel from '../AiPanel'
import { AI_PROVIDERS, AI_PROVIDER_ORDER, configProblem, useAiConfig } from '../../ai'

function showFile() {
  const label = WebviewWindow.getCurrent().label
  invoke("show_file_by_label", {label})
}

async function saveFile(content: string) {
  let defaultPath = await WebviewWindow.getCurrent().title()
  for (let name of ["server_log", "client_log", "log"]) {
    if (defaultPath.startsWith(name)) {
      defaultPath = name + ".txt"
      break
    }
  }
  invoke("save_file", { defaultPath, content })
}

type SidePanelProps = {
  logContent: string,
  logComment: LogComment,
  navigate: (id: NavigateAction)=> void,
}

export default function SidePanel(props: SidePanelProps) {
  const comment = props.logComment || {} as LogComment
  const { navigate, logContent } = props
  const [showHelpHint, setShowHelpHint] = useState(true)
  const { t } = useLingui()
  const hasBug = comment.has_lua_crash || comment.has_c_crash // TODO: use field finding?

  const [showModList, setShowModList] = useState(false)
  const [showGameInfo, setShowGameInfo] = useState(false)
  const [showAi, setShowAi] = useState(false)
  const [aiConfig, updateAiConfig] = useAiConfig()
  const aiProblem = configProblem(aiConfig)

  return (
    <div className="h-screen bg-blue-50 p-2 select-none text-gray-600"
      style={{minWidth: 200, maxWidth: 200}}>
      <h1 className="font-bold">{t`QUICK COMMANDS`}</h1>
      <div className="h-screen overflow-auto">
        <Section title={t`Go to...`}/>
        <div className="relative bg-red-500/5 border-red-500 text-red-500 select-auto border-1 rounded-sm \
          text-sm my-2 p-2 pr-1.5"
          style={{display: showHelpHint && hasBug ? "block" : "none"}}>
          <div className="absolute right-px px-1 top-px cursor-pointer hover:bg-black/10"
            onClick={()=> setShowHelpHint(false)}>×</div>
          {t`This log contains error messages, click the button below to view.`}
        </div>
        <Button disable={!hasBug} onClick={()=> navigate("prev-error")}>{t`Prev error info`}</Button>
        <Button disable={!hasBug} onClick={()=> navigate("next-error")}>{t`Next error info`}</Button>
        <Button onClick={()=> navigate("prev-instance")}>{t`Prev game instance`}</Button>
        <Button onClick={()=> navigate("next-instance")}>{t`Next game instance`}</Button>
        <Section title={t`Show info...`} />
        <Button onClick={()=> setShowGameInfo(v=> !v)}>{t`Game info`}</Button>
        {
          showGameInfo && <GameInfo comment={comment}/>
        }
        <Button onClick={()=> setShowModList(v=> !v)}>{t`Mod list`}</Button>
        {
          showModList && comment.mods && <ModList comment={comment}/>
        }
        <Section title={t`File operation...`}/>
        <Button onClick={showFile}>{t`Reveal in folder`}</Button>
        <Button onClick={()=> saveFile(logContent)}>{t`Save as`}</Button>
        <Section title={t`AI analysis`}/>
        <AiConfigFields config={aiConfig} onChange={updateAiConfig}/>
        <button
          disabled={!!aiProblem}
          onClick={()=> setShowAi(true)}
          className="mt-1 block w-full rounded-sm border border-blue-400 bg-blue-100 px-2 py-1             text-sm text-blue-700 transition-all cursor-pointer hover:bg-blue-200             disabled:cursor-not-allowed disabled:border-slate-300 disabled:bg-slate-100             disabled:text-slate-400">
          {t`Analyze with AI`}
        </button>
        {
          aiProblem &&
          <p className="mt-0.5 text-[10px] text-red-400">
            {aiProblem === "apiKey"
              ? t`请填写 API Key`
              : aiProblem === "model" ? t`请填写模型名称` : t`请填写接口域名`}
          </p>
        }
        <div className="flex justify-end">
          <span
            onClick={()=> invoke("open_settings_window")}
            className="cursor-pointer text-[10px] text-gray-400 underline hover:text-blue-400">
            {t`More settings`}
          </span>
        </div>
        <div className="h-40"></div>
      </div>
      <AiPanel open={showAi} onClose={()=> setShowAi(false)}/>
    </div>
  )
}

type AiConfigFieldsProps = {
  config: ReturnType<typeof useAiConfig>[0],
  onChange: ReturnType<typeof useAiConfig>[1],
}

const aiInputClass = "w-full rounded-sm border border-slate-300 bg-white/90 px-1.5 py-1 \
  text-xs text-gray-600 outline-none focus:border-blue-400"

function AiConfigFields(props: AiConfigFieldsProps) {
  const { config, onChange } = props
  const { t } = useLingui()
  const preset = AI_PROVIDERS[config.provider]
  return (
    <div className="mt-1 space-y-1 select-text">
      <select
        value={config.provider}
        onChange={e=> onChange({provider: e.target.value as any})}
        className={aiInputClass + " cursor-pointer"}>
        {
          AI_PROVIDER_ORDER.map(provider=> 
            <option key={provider} value={provider}>{AI_PROVIDERS[provider].label}</option>
          )
        }
      </select>
      <input
        type="password"
        value={config.apiKey}
        spellCheck={false}
        autoComplete="off"
        placeholder={t`API Key`}
        onChange={e=> onChange({apiKey: e.target.value})}
        className={aiInputClass}
      />
      <input
        type="text"
        value={config.model}
        spellCheck={false}
        placeholder={preset.model || t`模型名称`}
        onChange={e=> onChange({model: e.target.value})}
        className={aiInputClass}
      />
      {
        preset.needBaseUrl &&
        <input
          type="text"
          value={config.baseUrl}
          spellCheck={false}
          placeholder={t`接口域名`}
          onChange={e=> onChange({baseUrl: e.target.value})}
          className={aiInputClass}
        />
      }
    </div>
  )
}

type ButtonProps = {
  intent?: "danger" | "warning" | "success" | "ai",
  disable?: boolean,
  children: React.ReactNode,
  onClick?: ()=> void
}

function Section(props: {title: string}) {
  return (
    <div>
      <div className="h-px bg-slate-300 my-2"/>
      <h2>{props.title}</h2>
    </div>
  )
}

function Button(props: ButtonProps) {
  const {disable} = props
  let colorClass = "bg-white/90 hover:bg-slate-100 border-slate-300"
  if (props.intent === "danger") {
    colorClass = "bg-red-100 hover:bg-red-200 border-red-500"
  }
  else if (props.intent === "ai") {
    colorClass = "bg-blue-100 hover:bg-blue-200 border-blue-400 text-blue-700"
  }
  return (
    <button
      disabled={disable} 
      className={[colorClass, " transition-all block px-2 py-1 my-0.5 text-sm whitespace-nowrap \
        border rounded-sm bg cursor-pointer \
        disabled:cursor-not-allowed disabled:opacity-50"].join(" ")}
      onClick={!disable && props.onClick}>
      {props.children}
    </button>
  )
}

type GameInfoProps = {
  comment: LogComment
}

function GameInfo(props: GameInfoProps) {
  const {comment} = props

  return (
    <div className="bg-white/90 border-slate-300 border rounded-sm p-2 my-0.5 text-sm \
      max-w-full break-words select-text">
      <p>version: {comment.build_version}</p>
      <p>platform: {comment.build_platform}</p>
      <p>arch: {comment.build_arch}</p>
      <p>run time: {formatRunTime(comment.total_time)}</p>
      {/* <p>file mounting: </p> */}
      {/* {JSON.stringify(comment.databundles_mounting_state)} */}
    </div>
  )
}

type ModListProps = {
  comment: LogComment
}

function ModList(props: ModListProps) {
  const {comment} = props
  const {t} = useLingui()
  const mods = comment.mods

  return (
    <div className="bg-white/90 border-slate-300 border rounded-sm p-2 my-0.5 text-sm \
      max-w-full break-words select-text">
      {
        mods.length === 0 && t`No mod found`
      }
      {
        mods.map((mod, i)=>
          <div key={mod.moddir} className="">
            <p className="mb-1">
              <span className="font-bold">
                {mod.name}
              </span>
              <span className="opacity-70 ml-2 font-mono">
                {mod.version}
              </span>
            </p>
            <p className={mod.workshop_id ? "block" : "hidden"}>
              <span
                onClick={()=> window.visitMod(mod.workshop_id)}
                className="text-sm underline cursor-pointer hover:text-blue-400">
                {mod.workshop_id}
              </span>
            </p>
            {
              i !== mods.length - 1 && <div className="h-px bg-slate-300 my-1"></div>
            }
          </div>
        )
      }
    </div>
  )
}