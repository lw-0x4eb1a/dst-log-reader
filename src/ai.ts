// AI analysis configuration, stored in localStorage (shared by every window)

import { useSetting } from "./hooks"

export type AiProvider = "deepseek" | "claude" | "openai" | "custom"

export type AiConfig = {
  provider: AiProvider,
  apiKey: string,
  model: string,
  baseUrl: string,
  gameDir: string,
}

export type AiCause = {
  title: string,
  detail: string,
  mods?: string[],
}

export type AiMod = {
  moddir?: string,
  name: string,
  reason: string,
}

export type AiResult = {
  summary: string,
  causes: AiCause[],
  mods: AiMod[],
  suggestions: string[],
  raw?: boolean,
}

export type AiUsage = {
  prompt_tokens: number,
  completion_tokens: number,
  total_tokens: number,
  cached_tokens: number,
  context_tokens: number,
  context_limit?: number | null,
  turns: number,
}

export type AiState =
  | { state: "idle" }
  | { state: "running", status: string, tail: string, startedAt: number, usage?: AiUsage }
  | { state: "done", result: AiResult, usage?: AiUsage }
  | { state: "error", message: string, needsConfig?: boolean }

/** React hook backed by localStorage, shared by the sidebar and settings */
export function useAiConfig() {
  const [provider, setProvider] = useSetting(AI_KEYS.provider, "deepseek")
  const [apiKey, setApiKey] = useSetting(AI_KEYS.apiKey, "")
  const [model, setModel] = useSetting(AI_KEYS.model, "")
  const [baseUrl, setBaseUrl] = useSetting(AI_KEYS.baseUrl, "")
  const [gameDir, setGameDir] = useSetting(AI_KEYS.gameDir, "")
  const config: AiConfig = {
    provider: (AI_PROVIDERS[provider as AiProvider] ? provider : "deepseek") as AiProvider,
    apiKey, model, baseUrl, gameDir,
  }
  const update = (patch: Partial<AiConfig>)=> {
    if (patch.provider !== undefined) {
      setProvider(patch.provider)
      const defaults = AI_PROVIDER_ORDER.map(p=> AI_PROVIDERS[p].model)
      // follow the new default unless the user typed their own model name
      if (!model.trim() || defaults.includes(model)) {
        setModel(AI_PROVIDERS[patch.provider].model)
      }
    }
    if (patch.apiKey !== undefined) setApiKey(patch.apiKey)
    if (patch.model !== undefined) setModel(patch.model)
    if (patch.baseUrl !== undefined) setBaseUrl(patch.baseUrl)
    if (patch.gameDir !== undefined) setGameDir(patch.gameDir)
  }
  return [config, update] as const
}

/** short label for a token count */
export function formatTokens(value?: number) {
  if (!value) return "0"
  if (value >= 1_000_000) return (value / 1_000_000).toFixed(2) + "M"
  if (value >= 10_000) return (value / 1000).toFixed(0) + "K"
  if (value >= 1000) return (value / 1000).toFixed(1) + "K"
  return String(value)
}

/** event emitted by the rust side while analyzing */
export const AI_EVENT_NAME = "ai-analysis"

export const AI_KEYS = {  provider: "ai_provider",
  apiKey: "ai_api_key",
  model: "ai_model",
  baseUrl: "ai_base_url",
  gameDir: "ai_game_dir",
}

export type ProviderPreset = {
  label: string,
  model: string,
  baseUrl: string,
  needBaseUrl?: boolean,
  note?: string,
}

export const AI_PROVIDERS: Record<AiProvider, ProviderPreset> = {
  deepseek: {
    label: "DeepSeek",
    model: "deepseek-flash",
    baseUrl: "https://api.deepseek.com",
    note: "deepseek-flash（默认）/ deepseek-v4-pro，支持思考与工具调用",
  },
  claude: {
    label: "Claude",
    model: "claude-sonnet-4-5",
    baseUrl: "https://api.anthropic.com",
    note: "claude-sonnet-4-5 / claude-3-5-haiku-latest",
  },
  openai: {
    label: "ChatGPT",
    model: "gpt-4o",
    baseUrl: "https://api.openai.com/v1",
    note: "gpt-4o / gpt-4o-mini",
  },
  custom: {
    label: "自定义",
    model: "",
    baseUrl: "",
    needBaseUrl: true,
    note: "OpenAI 兼容接口，例如 https://your-domain/v1",
  },
}

export const AI_PROVIDER_ORDER: AiProvider[] = ["deepseek", "claude", "openai", "custom"]

export function loadAiConfig(): AiConfig {
  const provider = (localStorage.getItem(AI_KEYS.provider) || "deepseek") as AiProvider
  return {
    provider: AI_PROVIDERS[provider] ? provider : "deepseek",
    apiKey: localStorage.getItem(AI_KEYS.apiKey) || "",
    model: localStorage.getItem(AI_KEYS.model) || "",
    baseUrl: localStorage.getItem(AI_KEYS.baseUrl) || "",
    gameDir: localStorage.getItem(AI_KEYS.gameDir) || "",
  }
}

export function saveAiConfig(config: AiConfig) {
  localStorage.setItem(AI_KEYS.provider, config.provider)
  localStorage.setItem(AI_KEYS.apiKey, config.apiKey)
  localStorage.setItem(AI_KEYS.model, config.model)
  localStorage.setItem(AI_KEYS.baseUrl, config.baseUrl)
  localStorage.setItem(AI_KEYS.gameDir, config.gameDir)
}

/** the effective model: what the user typed, or the provider default */
export function effectiveModel(config: AiConfig) {
  return (config.model || AI_PROVIDERS[config.provider]?.model || "").trim()
}

export function configProblem(config: AiConfig): string {
  if (!config.apiKey.trim()) {
    return "apiKey"
  }
  if (!effectiveModel(config)) {
    return "model"
  }
  if (config.provider === "custom" && !config.baseUrl.trim()) {
    return "baseUrl"
  }
  return ""
}

/** localStorage keys used by the settings window */
export const AI_SETTING_KEYS = [
  AI_KEYS.provider,
  AI_KEYS.apiKey,
  AI_KEYS.model,
  AI_KEYS.baseUrl,
  AI_KEYS.gameDir,
]

/** map a moddir / mod name from the AI answer to the loaded mod list */
export function resolveMod(moddirOrName: string) {
  const list = window.globalModList || []
  const key = (moddirOrName || "").trim()
  const found = list.find(v=> v.moddir === key)
    || list.find(v=> v.name === key)
    || list.find(v=> key.length > 0 && (v.name || "").includes(key))
  const workshop = found?.workshop_id
    || (/^workshop-\d+$/.test(key) ? key.slice("workshop-".length) : "")
  const isId = /^\d{6,15}$/.test(workshop)
  return {
    label: found ? `${found.name}${found.version ? " v" + found.version : ""}` : key,
    workshopId: isId ? workshop : "",
  }
}
