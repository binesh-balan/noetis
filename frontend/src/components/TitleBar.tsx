'use client'

import { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Minus, Square, Copy, X } from 'lucide-react'

// The main window has no native frame on Windows (tauri.windows.conf.json); this replaces it.
// ponytail: Windows only; macOS/Linux keep their native title bars.
export function TitleBar() {
  const [shown, setShown] = useState(false) // decided after mount: the static export has no navigator
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    if (!navigator.userAgent.includes('Windows')) return
    setShown(true)
    const win = getCurrentWindow()
    const sync = () => win.isMaximized().then(setMaximized).catch(() => undefined)
    sync()
    const unlisten = win.onResized(sync)
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  if (!shown) return null
  const win = () => getCurrentWindow()

  return (
    <header
      data-tauri-drag-region
      className="relative z-[100] flex h-9 shrink-0 select-none items-center border-b border-border bg-sidebar"
    >
      <div data-tauri-drag-region className="pointer-events-none flex items-center gap-2 pl-3">
        <img src="/icon_32x32@2x.png" alt="" className="h-4 w-4 rounded-[3px]" />
        <span className="text-xs font-medium tracking-wide text-foreground/80">Noetis</span>
      </div>
      <div data-tauri-drag-region className="h-full flex-1" />
      <div className="flex h-full">
        <WindowButton label="Minimize" onClick={() => win().minimize()}>
          <Minus className="h-4 w-4" strokeWidth={1.5} />
        </WindowButton>
        <WindowButton label={maximized ? 'Restore' : 'Maximize'} onClick={() => win().toggleMaximize()}>
          {maximized ? (
            <Copy className="h-3.5 w-3.5 -scale-x-100" strokeWidth={1.5} />
          ) : (
            <Square className="h-3 w-3" strokeWidth={1.5} />
          )}
        </WindowButton>
        <WindowButton label="Close" onClick={() => win().close()} danger>
          <X className="h-4 w-4" strokeWidth={1.5} />
        </WindowButton>
      </div>
    </header>
  )
}

function WindowButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string
  onClick: () => void
  danger?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`flex h-full w-11 items-center justify-center text-foreground/70 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${
        danger ? 'hover:bg-destructive hover:text-destructive-foreground' : 'hover:bg-foreground/10 hover:text-foreground'
      }`}
    >
      {children}
    </button>
  )
}
