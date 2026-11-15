import { useEffect, useId, useRef } from "react"
import { useMutation } from "@tanstack/react-query"
import { Button } from "@/components/ui/button"
import { message } from "@/lib/api"
import { useStatus } from "./status-bar"
export function ConfirmAction({
  label,
  description,
  onConfirm,
}: {
  label: string
  description: string
  onConfirm: () => Promise<unknown>
}) {
  const { notice, report, clear } = useStatus()
  const hint = useId()
  const button = useRef<HTMLButtonElement>(null)
  const armed = useRef<{ id: number; until: number } | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(
    () => () => {
      clearTimeout(timer.current)
      if (armed.current) clear(armed.current.id)
    },
    [clear]
  )
  const action = useMutation({
    mutationFn: onConfirm,
    onSuccess: () => report(`Completed: ${label}`),
    onError: (error) => report(`${label} failed: ${message(error)}`, "error"),
  })
  return (
    <>
      <Button
        ref={button}
        variant="destructive"
        disabled={action.isPending}
        aria-describedby={hint}
        onClick={() => {
          clearTimeout(timer.current)
          if (
            armed.current &&
            notice?.id === armed.current.id &&
            Date.now() < armed.current.until
          ) {
            armed.current = null
            report(`Working: ${label}`)
            action.mutate()
            return
          }
          const id = report(
            `Press ${label} again within 3 seconds to confirm. ${description}`,
            "attention",
            () => button.current?.click()
          )
          armed.current = { id, until: Date.now() + 3_000 }
          timer.current = setTimeout(() => {
            armed.current = null
            clear(id)
          }, 3_000)
        }}
      >
        {action.isPending ? "Working…" : label}
      </Button>
      <span id={hint} className="sr-only">
        {description} Press twice within 3 seconds to confirm.
      </span>
    </>
  )
}
