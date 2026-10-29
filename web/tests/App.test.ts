import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../src/App.vue'
import type { Operation } from '../src/api'
const plugin = { id: 'echo-plugin', version: '1.0.0', state: 'active', generation: 1, error: null }
const snapshot = { snapshot: { revision: 4, config_version: 1, values: { prefix: 'hello', auth: { $secret: 'env:TEST_TOKEN' } } }, status: { desired_revision: 4, applied_revision: 4 }, schema: { type: 'object', properties: { prefix: { type: 'string' } } } }
const wrappers: VueWrapper[] = []
let readonly = false, expired = false, loginRequired = false, conflict = false
let ops: Operation[] = [], calls: { method: string; payload: Record<string, unknown> }[] = []
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status })
function button(wrapper: VueWrapper, text: string) { const found = wrapper.findAll('button').find(b => b.text() === text); if (!found) throw new Error(`Missing button: ${text}`); return found }
async function openEditor(wrapper: VueWrapper) { await wrapper.get('.plugin-card').trigger('click'); await flushPromises(); return wrapper.get('textarea') }
async function setup() { const wrapper = mount(App); wrappers.push(wrapper); await flushPromises(); return wrapper }
beforeEach(() => {
  vi.stubGlobal('localStorage', { setItem: vi.fn(), getItem: vi.fn() }); vi.stubGlobal('sessionStorage', { setItem: vi.fn(), getItem: vi.fn() })
  vi.useFakeTimers(); readonly = false; expired = false; loginRequired = false; conflict = false; ops = []; calls = []
  vi.stubGlobal('fetch', vi.fn(async (path: string, init: RequestInit) => {
    if (path === '/api/v1/session') {
      if (init.method === 'DELETE') return new Response(null, { status: 204 })
      if (init.method === 'GET' && loginRequired) return response({ error: { code: 'Unauthorized', message: 'expired' } }, 401)
      return response({ csrf_token: 'csrf-fixture', expires_in_secs: 600 })
    }
    if (expired) return response({ error: { code: 'Unauthorized', message: 'expired' } }, 401)
    const body = JSON.parse(String(init.body)); calls.push(body)
    if (body.method === 'plugins.list') return response({ result: [plugin] })
    if (body.method === 'plugins.installed') return response({ result: [{ id: plugin.id, version: plugin.version, path: '/fixture', checksum: 'x' }, { id: 'other-plugin', version: '2.0.0', path: '/other', checksum: 'y' }] })
    if (body.method === 'operations.list') return response({ result: ops })
    if (body.method === 'management.overview') return response({ result: { permissions: { read: true, write: !readonly }, readonly } })
    if (body.method === 'config.schema' || body.method === 'config.get') return response({ result: snapshot })
    if (body.method === 'config.validate') return response({ result: { valid: true } })
    if (body.method === 'plugins.inspect') return response({ result: { manifest: { capabilities: ['events.subscribe', 'actions.call'], required_capabilities: ['events.subscribe'] } } })
    if (body.method === 'config.update' && conflict) return response({ error: { code: 'Conflict', message: 'management request conflicts' } }, 409)
    if (body.method === 'adapter.status') return response({ error: { code: 'Unsupported', message: 'unknown mock method' } }, 400)
    if (body.method === 'diagnostics.logs') return response({ result: { entries: [] } })
    return response({ result: { id: 'op-test', state: 'running' } })
  }))
})
afterEach(() => { wrappers.splice(0).forEach(w => w.unmount()); vi.useRealTimers(); vi.unstubAllGlobals() })
describe('management interactions', () => {
  it('logs in without storing or rendering the token', async () => {
    loginRequired = true
    const wrapper = await setup()
    const token = 'x'.repeat(40)
    await wrapper.get('#token').setValue(token)
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(wrapper.find('.app-layout').exists()).toBe(true)
    expect(localStorage.setItem).not.toHaveBeenCalled(); expect(sessionStorage.setItem).not.toHaveBeenCalled()
    expect(wrapper.html()).not.toContain(token)
  })
  it('sends the base revision and preserves draft through a direct conflict', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    const draft = '{"prefix":"edited","auth":{"$secret":"env:TEST_TOKEN"}}'
    await area.setValue(draft); conflict = true
    await button(wrapper, '保存并应用').trigger('click'); await flushPromises()
    expect(wrapper.get('[role="dialog"]').text()).toContain('修订 4')
    await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'config.update')?.payload).toMatchObject({ expected_revision: 4, values: JSON.parse(draft) })
    expect((area.element as HTMLTextAreaElement).value).toBe(draft)
    expect(wrapper.text()).toContain('输入已保留')
    expect(wrapper.text()).toContain('编辑基于修订 4')
  })
  it('polls async failure and keeps both text and base revision while showing Conflict inline', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    await area.setValue('{"prefix":"kept"}')
    await button(wrapper, '保存并应用').trigger('click'); await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    ops = [{ id: 'op-test', method: 'config.update', target: plugin.id, state: 'failed', stage: 'failed', result: { status: 'error', code: 'conflict' } }]
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect((area.element as HTMLTextAreaElement).value).toBe('{"prefix":"kept"}')
    expect(wrapper.text()).toContain('配置修订冲突。编辑内容和基准修订已保留')
    expect(wrapper.text()).toContain('编辑基于修订 4')
  })
  it('blocks malformed JSON and redacted placeholders before any HTTP mutation', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    await area.setValue('{"token":"***"}'); await button(wrapper, '保存并应用').trigger('click')
    expect(wrapper.text()).toContain('脱敏占位符'); expect(calls.some(c => c.method === 'config.update')).toBe(false)
    await area.setValue('{'); await button(wrapper, '校验配置').trigger('click')
    expect(wrapper.text()).toContain('JSON 格式有误'); expect(calls.some(c => c.method === 'config.validate')).toBe(false)
  })
  it('requires explicit replacement of a dirty draft', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    await area.setValue('{"prefix":"unsaved"}'); await button(wrapper, '重新读取配置').trigger('click')
    expect(wrapper.get('[role="dialog"]').text()).toContain('替换当前编辑内容')
    await button(wrapper, '取消').trigger('click')
    expect((area.element as HTMLTextAreaElement).value).toContain('unsaved')
  })
  it('shows readonly permissions and disables every mutation while leaving validation available', async () => {
    readonly = true
    const wrapper = await setup()
    expect(wrapper.text()).toContain('只读模式')
    await openEditor(wrapper)
    for (const text of ['＋ 安装本地插件', '启动', '停止', '重载此版本', '停用插件', '卸载此版本', '保存并应用']) expect(button(wrapper, text).attributes('disabled')).toBeDefined()
    expect(button(wrapper, '校验配置').attributes('disabled')).toBeUndefined()
  })
  it('requires confirmation and prevents double submission of privileged operations', async () => {
    const wrapper = await setup(); await openEditor(wrapper)
    await button(wrapper, '停止').trigger('click')
    expect(calls.some(c => c.method === 'plugins.stop')).toBe(false)
    const confirm = button(wrapper, '确认操作')
    await Promise.all([confirm.trigger('click'), confirm.trigger('click')]); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.stop')).toHaveLength(1)
    expect(calls.find(c => c.method === 'plugins.stop')?.payload.request_key).toBeTypeOf('string')
  })
  it('expires to the login page and removes loaded configuration', async () => {
    const wrapper = await setup(); const area = await openEditor(wrapper)
    await area.setValue('{"prefix":"sensitive-draft"}'); expired = true
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect(wrapper.find('#token').exists()).toBe(true)
    expect(wrapper.text()).toContain('会话已过期')
    expect(wrapper.html()).not.toContain('sensitive-draft')
  })
  it('polls runtime state without repeatedly rescanning installed packages', async () => {
    const wrapper = await setup()
    expect(calls.filter(c => c.method === 'plugins.installed')).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(5000); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.list').length).toBeGreaterThan(1)
    expect(calls.filter(c => c.method === 'plugins.installed')).toHaveLength(1)
    await button(wrapper, '↻ 刷新状态').trigger('click'); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.installed')).toHaveLength(2)
  })
  it('keeps adapter errors visible when diagnostic logs refresh', async () => {
    const wrapper = await setup()
    await wrapper.get('nav').findAll('button')[3].trigger('click'); await flushPromises()
    await wrapper.get('#adapter-id').setValue('echo-plugin')
    await button(wrapper, '查询状态').trigger('click'); await flushPromises()
    expect(wrapper.get('.adapter-error').text()).toContain('Unsupported')
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect(wrapper.get('.adapter-error').text()).toContain('Unsupported')
  })
  it('uses manifest capability checkboxes for explicit enable grants', async () => {
    const wrapper = await setup()
    await wrapper.get('.plugin-card').trigger('click'); await flushPromises()
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    const otherRow = wrapper.findAll('tbody tr').find(row => row.text().includes('other-plugin'))!
    await otherRow.get('button').trigger('click'); await flushPromises()
    await button(wrapper, '授权并启用').trigger('click'); await flushPromises()
    const checkboxes = wrapper.findAll('.grants input')
    expect(checkboxes[0].attributes('disabled')).toBeDefined()
    await checkboxes[1].setValue(true)
    await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'plugins.enable')?.payload.grants).toEqual(['events.subscribe', 'actions.call'])
  })
})
