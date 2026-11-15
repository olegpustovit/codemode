import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import type { Workspace } from "./queries"
import { nothing, write } from "@/lib/api"
import { ErrorNotice } from "@/components/common"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel } from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"

export function ToolScopeMode({ workspace }: { workspace: Workspace }) {
  const [mode, setMode] = useState(workspace.allTools ? "all" : "selected")
  const cache = useQueryClient()
  const save = useMutation({
    mutationFn: () =>
      write(
        `/api/workspaces/${workspace.id}/scope`,
        "PUT",
        { allTools: mode === "all" },
        nothing
      ),
    onSuccess: async () => {
      await Promise.all([
        cache.invalidateQueries({ queryKey: ["workspaces"] }),
        cache.invalidateQueries({ queryKey: ["policies", workspace.id] }),
      ])
    },
  })
  return (
    <form
      className="scope-controls"
      onSubmit={(event) => {
        event.preventDefault()
        save.mutate()
      }}
    >
      <Field className="max-w-md">
        <FieldLabel htmlFor="tool-scope-mode">Tool scope</FieldLabel>
        <NativeSelect
          id="tool-scope-mode"
          value={mode}
          onChange={(event) => setMode(event.target.value)}
          disabled={save.isPending}
        >
          <NativeSelectOption value="all">
            All tools from included sources
          </NativeSelectOption>
          <NativeSelectOption value="selected">
            Only selected tools
          </NativeSelectOption>
        </NativeSelect>
      </Field>
      <p className="max-w-prose text-sm text-muted-foreground">
        Selected scope hides tools until you include them; new tools stay
        hidden.
      </p>
      <ErrorNotice error={save.error} />
      <Button
        className="self-start"
        variant="outline"
        type="submit"
        disabled={
          save.isPending || mode === (workspace.allTools ? "all" : "selected")
        }
      >
        {save.isPending ? "Saving…" : "Save tool scope"}
      </Button>
    </form>
  )
}
