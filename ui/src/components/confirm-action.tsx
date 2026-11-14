import { useState } from "react"
import { useMutation } from "@tanstack/react-query"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { ErrorNotice } from "./common"
export function ConfirmAction({
  label,
  description,
  onConfirm,
}: {
  label: string
  description: string
  onConfirm: () => Promise<unknown>
}) {
  const [open, setOpen] = useState(false)
  const action = useMutation({
    mutationFn: onConfirm,
    onSuccess: () => setOpen(false),
  })
  return (
    <AlertDialog
      open={open}
      onOpenChange={(value) => {
        if (!action.isPending) {
          setOpen(value)
          action.reset()
        }
      }}
    >
      <AlertDialogTrigger render={<Button variant="destructive" />}>
        {label}
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{label}?</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        <ErrorNotice error={action.error} />
        <AlertDialogFooter>
          <AlertDialogCancel disabled={action.isPending}>
            Cancel
          </AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={action.isPending}
            onClick={() => action.mutate()}
          >
            {action.isPending ? "Working…" : label}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
