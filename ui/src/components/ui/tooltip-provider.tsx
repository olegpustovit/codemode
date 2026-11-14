import { Tooltip } from "@base-ui/react/tooltip"

export function TooltipProvider({
  delay = 0,
  ...props
}: Tooltip.Provider.Props) {
  return <Tooltip.Provider delay={delay} {...props} />
}
