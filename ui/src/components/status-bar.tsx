import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react"
import type { ReactNode, Ref } from "react"

type Tone = "normal" | "attention" | "error"
type Notice = { id: number; text: string; tone: Tone; action?: () => void }
type Status = {
  notice: Notice | null
  report: (text: string, tone?: Tone, action?: () => void) => number
  clear: (id: number) => void
}
const Context = createContext<Status | null>(null)
export function StatusProvider({ children }: { children: ReactNode }) {
  const [notice, setNotice] = useState<Notice | null>(null)
  const sequence = useRef(0)
  const report = useCallback(
    (text: string, tone: Tone = "normal", action?: () => void) => {
      const id = ++sequence.current
      setNotice({ id, text, tone, action })
      return id
    },
    []
  )
  const clear = useCallback((id: number) => {
    setNotice((current) => (current?.id === id ? null : current))
  }, [])
  const value = useMemo(
    () => ({ notice, report, clear }),
    [notice, report, clear]
  )
  return <Context.Provider value={value}>{children}</Context.Provider>
}
export function useStatus() {
  const status = useContext(Context)
  if (!status) throw new Error("StatusProvider is missing")
  return status
}
export function StatusBar({
  children,
  keysRef,
}: {
  children?: ReactNode
  keysRef?: Ref<HTMLDivElement>
}) {
  const { notice } = useStatus()
  return (
    <footer className="status-bar" aria-label="Status">
      <output
        className="status-message"
        aria-live="polite"
        aria-atomic="true"
        data-tone={notice?.tone}
      >
        {notice?.action ? (
          <button onClick={notice.action}>{notice.text}</button>
        ) : (
          (notice?.text ?? "Ready")
        )}
      </output>
      <div id="status-keys" className="status-keys" ref={keysRef}>
        {children}
      </div>
    </footer>
  )
}
