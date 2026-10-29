import { expect, test, type Page } from '@playwright/test'
async function mockConsole(page: Page, readonly = false) {
  let loggedIn = false, stopped = false
  const calls: { method: string; payload: Record<string, unknown> }[] = []
  const ops: unknown[] = []
  const plugins = () => [{ id: 'echo-plugin', version: '1.0.0', state: stopped ? 'stopped' : 'active', generation: 3, error: null }, { id: 'mock-adapter', version: '0.1.0', state: 'active', generation: 1, error: null }, { id: 'web-admin', version: '0.1.0', state: 'active', generation: 1, error: null }]
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname
    const send = (data: unknown, status = 200) => route.fulfill({ status, json: data })
    if (path === '/api/v1/session') {
      if (request.method() === 'DELETE') { loggedIn = false; await route.fulfill({ status: 204 }); return }
      if (request.method() === 'POST') loggedIn = true
      await send(loggedIn ? { csrf_token: 'browser-csrf', expires_in_secs: 600 } : { error: { code: 'Unauthorized', message: 'expired' } }, loggedIn ? 200 : 401); return
    }
    expect(request.headers()['x-csrf-token']).toBe('browser-csrf')
    expect(new URL(request.url()).search).toBe('')
    const body = request.postDataJSON(); calls.push(body)
    let result: unknown = null
    switch (body.method) {
      case 'plugins.list': result = plugins(); break
      case 'plugins.installed': result = plugins().map(p => ({ id: p.id, version: p.version, path: '/fixtures', checksum: 'fixture' })); break
      case 'management.overview': result = { permissions: { read: true, write: !readonly }, readonly }; break
      case 'operations.list': result = ops; break
      case 'config.schema': case 'config.get': result = { snapshot: { revision: 7, config_version: 1, values: { prefix: '你好，TwinTail', auth: { $secret: 'env:TEST_TOKEN' } } }, status: { desired_revision: 7, applied_revision: 7 }, schema: { type: 'object', properties: { prefix: { type: 'string', description: '回复前缀' }, auth: { description: '保留秘密引用' } } } }; break
      case 'config.validate': result = { valid: true }; break
      case 'config.update': ops.push({ id: 'op-conflict', target: 'echo-plugin', method: 'config.update', state: 'failed', stage: 'failed', created_unix_ms: Date.now(), result: { status: 'error', code: 'conflict' } }); result = { id: 'op-conflict', state: 'running' }; break
      case 'plugins.stop': stopped = true; ops.push({ id: 'op-stop', target: 'echo-plugin', method: body.method, state: 'completed', stage: 'completed', created_unix_ms: Date.now(), result: { status: 'ok' } }); result = { id: 'op-stop', state: 'running' }; break
      case 'diagnostics.logs': result = { entries: [], limit: 128 }; break
      case 'adapter.status': result = { connected: true, generation: 1, connection_attempts: 1, pending_actions: 0 }; break
      default: await send({ error: { code: 'Unsupported', message: 'Unsupported fixture method' } }, 400); return
    }
    await send({ result })
  })
  return { calls }
}
async function login(page: Page) {
  await page.goto('/')
  await page.getByLabel('管理令牌', { exact: true }).fill('synthetic-browser-token-not-a-real-credential')
  await page.getByRole('button', { name: '进入控制台 →' }).click()
  await expect(page.getByRole('heading', { name: '运行概览', exact: true })).toBeVisible()
}
test('login, confirmed stop, CAS conflict preservation, diagnostics and responsive layout', async ({ page }, info) => {
  const errors: string[] = []; page.on('pageerror', e => errors.push(e.message))
  const { calls } = await mockConsole(page)
  await login(page)
  await page.screenshot({ path: `/tmp/twintail-p5-${info.project.name}-overview.png`, fullPage: true })
  expect(await page.evaluate(() => ({ local: localStorage.length, session: sessionStorage.length }))).toEqual({ local: 0, session: 0 })
  await page.getByRole('button', { name: /echo-plugin.*查看详情与配置/ }).click()
  const editor = page.getByRole('textbox', { name: 'JSON 配置编辑器' })
  await expect(editor).toBeVisible()
  await page.screenshot({ path: `/tmp/twintail-p5-${info.project.name}-config.png`, fullPage: true })
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await expect(page.getByRole('dialog')).toBeVisible()
  expect(calls.filter(c => c.method === 'plugins.stop')).toHaveLength(0)
  await page.getByRole('button', { name: '确认操作', exact: true }).click()
  await expect(page.getByRole('dialog')).toBeHidden()
  await expect(page.locator('.detail-panel .panel-heading')).toContainText('已停止')
  const draft = '{"prefix":"保留编辑","auth":{"$secret":"env:TEST_TOKEN"}}'
  await editor.fill(draft)
  await page.getByRole('button', { name: '校验配置', exact: true }).click()
  await expect(page.locator('.detail-panel [role="status"]')).toContainText('配置校验通过')
  await page.getByRole('button', { name: '保存并应用', exact: true }).click()
  await page.getByRole('button', { name: '确认操作', exact: true }).click()
  await expect(page.locator('.detail-panel [role="alert"]')).toContainText('配置修订冲突')
  await expect(editor).toHaveValue(draft)
  expect(calls.find(c => c.method === 'config.update')?.payload).toMatchObject({ expected_revision: 7, values: JSON.parse(draft) })
  await expect(page.locator('.config-layout label')).toContainText('修订 7')
  await page.getByRole('navigation').getByRole('button', { name: /安全诊断/ }).click()
  await page.getByLabel('适配器插件', { exact: true }).selectOption('mock-adapter')
  await page.getByRole('button', { name: '查询状态', exact: true }).click()
  await expect(page.locator('.diagnostic-json')).toContainText('"connected": true')
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  expect(errors).toEqual([])
})
test('read-only permissions disable mutations and self-stop displays CLI rescue', async ({ page }) => {
  await mockConsole(page, true); await login(page)
  await expect(page.getByText('只读模式', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: /web-admin.*查看详情与配置/ }).click()
  await expect(page.getByRole('button', { name: '停止', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '＋ 安装本地插件', exact: true })).toBeDisabled()
  await expect(page.getByText('此插件提供当前管理界面。', { exact: false })).toBeVisible()
})
