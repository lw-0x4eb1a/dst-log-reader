import { useCallback, useEffect } from 'react'
import { useSetting } from '../../hooks'
import { emit } from '@tauri-apps/api/event'
import { useLingui } from '@lingui/react/macro'

export default function SettingsPage() {
  const { i18n } = useLingui()
  return (
    <div className="p-4 text-gray-800">
      <h2 className="mb-1">语言 / Language</h2>
      <RadioGroup
        id="language"
        options={[
          { key: "zh", label: "中文" },
          { key: "en", label: "English" },
        ]}
        defaultValue={i18n.locale || "en"}
      />
    </div>
  )
}

type RadioOption = {
  key: string,
  label: any,
}

type RadioGroupProps = {
  id: string,
  options: RadioOption[],
  onChange?: (value: string)=> void,
  defaultValue?: string,
}

function RadioGroup(props: RadioGroupProps) {
  const {id, options, onChange, defaultValue} = props
  const [value, setValue] = useSetting(id, defaultValue)
  const selectItem = useCallback((key: string)=> {
    setValue(key)
  }, [])

  useEffect(()=> {
    emit("setting", {key: id, value})
    if (onChange) {
      onChange(value)
    }
  }, [id, value])

  return (
    <div className="grid gap-1">
      {
        options.map(({key, label})=> 
          <div 
            key={key} 
            onClick={()=> selectItem(key)}
            className="flex items-center p-2 space-x-2 cursor-pointer rounded-sm hover:bg-gray-100 transition-all">
            <button className="aspect-square w-4 h-4 rounded-full border focus:outline-none\
                relative cursor-pointer">
              {
                value === key && 
                <div className="absolute w-2.5 h-2.5 rounded-full bg-black left-0.5 top-0.5">

                </div>
              }
            </button>
            <label className="text-sm font-medium leading-none cursor-pointer">
              {label}
            </label>
          </div>
        )
      }
    </div>
  )

}