import { siGithub, siLinear, siNotion } from "simple-icons"
import {
  Plug,
  Database,
  Cloud,
  Zap,
  ChartColumn,
  Mail,
  Calendar,
  CreditCard,
  Box,
  Bug,
  BookOpen,
  Globe,
  Terminal,
  KeyRound,
  Users,
  File,
} from "lucide-react"
export const sourceIcons = {
  plug: Plug,
  database: Database,
  cloud: Cloud,
  bolt: Zap,
  chart: ChartColumn,
  mail: Mail,
  calendar: Calendar,
  card: CreditCard,
  box: Box,
  bug: Bug,
  book: BookOpen,
  globe: Globe,
  terminal: Terminal,
  key: KeyRound,
  users: Users,
  file: File,
}
export const sourceIconKeys = [...Object.keys(sourceIcons), "mono"]

const glyphs: Record<string, { path: string }> = {
  github: siGithub,
  linear: siLinear,
  notion: siNotion,
}
export function VendorGlyph({
  name,
  large = false,
  label,
}: {
  name: string
  large?: boolean
  label?: string
}) {
  const glyph = glyphs[name]
  const Icon =
    Object.entries(sourceIcons).find(([key]) => key === name)?.[1] ??
    (name === "custom" ? Plug : undefined)
  if (!large)
    return glyph ? (
      <svg
        aria-hidden="true"
        viewBox="0 0 24 24"
        data-icon={name}
        className="vendor-glyph"
      >
        <path d={glyph.path} fill="currentColor" />
      </svg>
    ) : Icon ? (
      <Icon
        aria-hidden="true"
        data-icon={name}
        className="vendor-glyph"
        strokeWidth={1.5}
      />
    ) : (
      <span
        data-icon={name}
        className="vendor-glyph vendor-initial"
        aria-hidden="true"
      >
        {(label ?? name)[0]?.toUpperCase()}
      </span>
    )
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 240 168"
      data-icon={name}
      className="vendor-art"
    >
      <g fill="none" stroke="currentColor">
        <circle cx="120" cy="84" r="72" strokeWidth=".8" opacity=".5" />
        <circle cx="120" cy="84" r="58" strokeWidth=".5" opacity=".25" />
        {Array.from({ length: 40 }, (_, index) => {
          const angle = (index / 40) * Math.PI * 2,
            radius = index % 5 ? 75 : 80
          return (
            <line
              key={index}
              x1={120 + 72 * Math.cos(angle)}
              y1={84 + 72 * Math.sin(angle)}
              x2={120 + radius * Math.cos(angle)}
              y2={84 + radius * Math.sin(angle)}
              strokeWidth=".7"
              opacity=".45"
            />
          )
        })}
        {glyph && (
          <g transform="translate(90 54) scale(2.5)">
            <path d={glyph.path} strokeWidth=".3" />
          </g>
        )}
        {Icon && (
          <Icon x={90} y={54} width={60} height={60} strokeWidth={0.7} />
        )}
      </g>
      {!glyph && !Icon && (
        <text x="120" y="98" textAnchor="middle" className="vendor-art-initial">
          {(label ?? name)[0]?.toUpperCase()}
        </text>
      )}
    </svg>
  )
}
