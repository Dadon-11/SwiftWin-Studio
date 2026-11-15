import type { EngineInterface, Register } from 'claude-code'

// Opens SwiftWin's live iPhone in the Claude app's Browser pane the first time Claude uses a
// swiftwin tool, so nobody has to ask for it. The link comes from show_phone: one-time, 2 minutes.
type Opened = { isShown: true } | { isShown: false; why: string }

const LINK = /http:\/\/127\.0\.0\.1:\d+\/#phone&pair=[A-Za-z0-9_-]+/

// shortcut: a module variable, so a reload of the mod opens the phone once more; fine for a viewer.
let isDone = false

async function openLink($: EngineInterface, text: string): Promise<Opened> {
  const url = text.match(LINK)?.[0]
  if (!url) return { isShown: false, why: 'show_phone gave no link' }
  const r = await $.mcp.call('Claude_Browser', 'preview_start', { url })
  if (r.isError) return { isShown: false, why: 'the Browser pane refused it' }
  // preview_start opens the tab with the pane still hidden; bring it into view (best effort).
  await $.mcp.call('ccd_view', 'show_pane', { pane: 'preview' }).catch(() => undefined)
  return { isShown: true }
}

const textOf = (content: readonly { type: string; text?: string }[]) => content.map(c => c.text ?? '').join('\n')

export const register: Register = on => {
  on('tool.call', async ($, e, next) => {
    if (!e.tool.startsWith('mcp__swiftwin__')) return next(e)

    if (e.tool === 'mcp__swiftwin__show_phone') {
      const ran = await next(e)
      if (ran.deny !== undefined || ran.isError) return ran
      const opened = await openLink($, JSON.stringify(ran.result)).catch(() => ({ isShown: false, why: 'the Browser pane is not reachable' }) as Opened)
      isDone ||= opened.isShown
      return opened.isShown
        ? { ...ran, context: ['The swiftwin-phone plugin already opened this link in the Browser pane: the user sees the live iPhone. Do not open it again.'] }
        : ran
    }

    if (isDone) return next(e)
    isDone = true
    const show = $.mcp.call('swiftwin', 'show_phone', {}).then(r => openLink($, textOf(r.content)))
    const [ran, opened] = await Promise.all([next(e), show.catch(() => ({ isShown: false, why: 'show_phone failed' }) as Opened)])
    if (opened.isShown || ran.deny !== undefined) return ran
    isDone = false
    return { ...ran, context: [...(ran.context ?? []), `The user cannot see the iPhone yet (${opened.why}): call show_phone and open its link with preview_start now, without asking.`] }
  }).catch(($, e, next) => next(e))
}
