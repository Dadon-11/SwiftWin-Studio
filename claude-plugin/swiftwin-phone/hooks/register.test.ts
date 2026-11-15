import { expect, test } from 'claude-code/testing'

const LINK = 'http://127.0.0.1:57270/#phone&pair=abc_DEF-123'

// Stands for the engine: swiftwin answers show_phone with `link`, the Browser pane records what it opened.
function engine(on: any, link: string) {
  const opened: string[] = []
  on('mcp.call', (_: unknown, e: { server: string; tool: string; args: { url?: string } }) => {
    if (e.server === 'swiftwin') return { value: { content: [{ type: 'text', text: `Open this link\n${link}` }], isError: false } }
    if (e.server === 'ccd_view') return { value: { content: [], isError: false } }
    opened.push(String(e.args.url))
    return { value: { content: [], isError: false } }
  })
  on('tool.call', () => ({ result: {} }))
  return opened
}

test('the first swiftwin tool opens the phone once; other tools do not', async ($, on) => {
  const opened = engine(on, LINK)
  await $.tool.call({ tool: 'Bash', command: 'ls' })
  expect(opened).toEqual([])
  await $.tool.call({ tool: 'mcp__swiftwin__screenshot' })
  await $.tool.call({ tool: 'mcp__swiftwin__tap', label: 'OK' })
  expect(opened).toEqual([LINK])
})

test('a link that is not SwiftWin loopback is never opened', async ($, on) => {
  const opened = engine(on, 'http://evil.example/#phone&pair=abc')
  const r = await $.tool.call({ tool: 'mcp__swiftwin__screenshot' })
  expect(opened).toEqual([])
  expect(JSON.stringify(r)).toContain('cannot see the iPhone yet')
})
