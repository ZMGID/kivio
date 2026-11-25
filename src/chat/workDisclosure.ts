import { createContext, useCallback, useContext, useState } from 'react'

// Each window remembers explicit choices across row recycling and chat switches.
// Tool and segment IDs are local to an assistant message.
export const WorkDisclosureScope = createContext<string | null>(null)
const choices = new Map<string, boolean>()
const maxChoices = 2048

function useDisclosureValue(kind: string, id: string | undefined, defaultOpen?: boolean) {
  const scope = useContext(WorkDisclosureScope)
  const key = JSON.stringify([scope, kind, id])
  const remember = Boolean(scope && id)
  const [local, setLocal] = useState<{ key: string; open: boolean }>()
  const open = local?.key === key ? local.open : (remember ? choices.get(key) : undefined)
  const setOpen = useCallback((next: boolean) => {
    if (remember) {
      choices.delete(key)
      if (next !== defaultOpen) choices.set(key, next)
      if (choices.size > maxChoices) choices.delete(choices.keys().next().value!)
    }
    setLocal({ key, open: next })
  }, [defaultOpen, key, remember])
  return [open, setOpen] as const
}

export function useWorkDisclosure(kind: string, id: string | undefined, defaultOpen = false) {
  const [choice, setOpen] = useDisclosureValue(kind, id, defaultOpen)
  return [choice ?? defaultOpen, setOpen] as const
}

export function useWorkDisclosureChoice(kind: string, id: string | undefined) {
  const [choice, setOpen] = useDisclosureValue(kind, id)
  return [choice ?? null, setOpen] as const
}
