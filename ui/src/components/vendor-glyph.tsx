import { siGithub, siLinear, siNotion, siSentry } from "simple-icons"
import { Plug } from "lucide-react"
const glyphs: Record<string, { path: string }> = {
  github: siGithub,
  linear: siLinear,
  notion: siNotion,
  sentry: siSentry,
}
export function VendorGlyph({
  name,
  large = false,
}: {
  name: string
  large?: boolean
}) {
  const glyph = glyphs[name]
  if (!large)
    return glyph ? (
      <svg aria-hidden="true" viewBox="0 0 24 24" className="vendor-glyph">
        <path d={glyph.path} fill="currentColor" />
      </svg>
    ) : name === "custom" ? (
      <Plug aria-hidden="true" className="vendor-glyph" strokeWidth={1.5} />
    ) : (
      <span className="vendor-glyph vendor-initial" aria-hidden="true">
        {name[0]?.toUpperCase()}
      </span>
    )
  return (
    <svg aria-hidden="true" viewBox="0 0 240 168" className="vendor-art">
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
        {name === "custom" && (
          <g transform="translate(90 54) scale(2.5)" strokeWidth=".5">
            <path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4" />
          </g>
        )}
      </g>
      {!glyph && name !== "custom" && (
        <text x="120" y="98" textAnchor="middle" className="vendor-art-initial">
          {name[0]?.toUpperCase()}
        </text>
      )}
    </svg>
  )
}
