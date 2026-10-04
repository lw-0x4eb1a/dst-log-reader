import type { ReactNode } from "react"
import { useLingui } from "@lingui/react/macro"
import {
  AI_PROVIDERS,
  AI_PROVIDER_ORDER,
  type AiConfig,
  type AiProvider,
} from "../../ai"

type AiConfigFieldsProps = {
  config: AiConfig,
  onChange: (config: AiConfig)=> void,
}

const inputClass = "w-full px-2 py-1 text-sm bg-white/90 border border-slate-300 rounded-sm \
  text-gray-700 outline-none focus:border-blue-400"

export default function AiConfigFields(props: AiConfigFieldsProps) {
  const { config, onChange } = props
  const { t } = useLingui()
  const preset = AI_PROVIDERS[config.provider]

  const update = (patch: Partial<AiConfig>)=> onChange({...config, ...patch})

  const changeProvider = (provider: AiProvider)=> {
    const patch: Partial<AiConfig> = { provider }
    // follow the new default unless the user typed a custom model
    const defaults = AI_PROVIDER_ORDER.map(p=> AI_PROVIDERS[p].model)
    if (!config.model.trim() || defaults.includes(config.model)) {
      patch.model = AI_PROVIDERS[provider].model
    }
    onChange({...config, ...patch})
  }

  return (
    <div className="space-y-2">
      <Field label={t`服务商`}>
        <div className="flex flex-wrap gap-1 mt-1">
          {
            AI_PROVIDER_ORDER.map(provider=> 
              <button
                key={provider}
                type="button"
                onClick={()=> changeProvider(provider)}
                className={[
                  "px-2 py-0.5 text-xs border rounded-sm cursor-pointer transition-all",
                  config.provider === provider
                    ? "bg-blue-500 border-blue-500 text-white"
                    : "bg-white/90 border-slate-300 text-gray-600 hover:bg-slate-100",
                ].join(" ")}>
                {AI_PROVIDERS[provider].label}
              </button>
            )
          }
        </div>
      </Field>
      <Field label={t`API Key`}>
        <input
          type="password"
          className={inputClass}
          value={config.apiKey}
          spellCheck={false}
          autoComplete="off"
          placeholder="sk-..."
          onChange={e=> update({apiKey: e.target.value})}
        />
      </Field>
      <Field label={t`模型名称`} hint={preset.note}>
        <input
          type="text"
          className={inputClass}
          value={config.model}
          spellCheck={false}
          placeholder={preset.model}
          onChange={e=> update({model: e.target.value})}
        />
      </Field>
      <Field
        label={preset.needBaseUrl ? t`接口域名` : t`接口域名（可选）`}
        hint={
          preset.needBaseUrl
            ? t`OpenAI 兼容接口，例如 https://your-domain/v1`
            : t`留空使用官方接口；也可填写代理地址`
        }>
        <input
          type="text"
          className={inputClass}
          value={config.baseUrl}
          spellCheck={false}
          placeholder={preset.baseUrl || "https://your-domain/v1"}
          onChange={e=> update({baseUrl: e.target.value})}
        />
      </Field>
      <Field label={t`游戏安装目录（可选）`} hint={t`留空自动检测；无法读取游戏源码时再填写`}>
        <input
          type="text"
          className={inputClass}
          value={config.gameDir}
          spellCheck={false}
          placeholder={t`例如 .../steamapps/common/Don't Starve Together`}
          onChange={e=> update({gameDir: e.target.value})}
        />
      </Field>
      <p className="text-[11px] text-gray-400">
        {t`配置只保存在本机，仅分析时发送给你选择的服务商。`}
      </p>
    </div>
  )
}

function Field(props: {label: string, hint?: string, children: ReactNode}) {
  return (
    <div>
      <label className="text-xs text-gray-500">{props.label}</label>
      {props.children}
      {props.hint && <p className="text-[10px] text-gray-400 mt-0.5">{props.hint}</p>}
    </div>
  )
}
