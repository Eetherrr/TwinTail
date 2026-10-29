import { cp, mkdir, readdir, rm } from 'node:fs/promises'
const target = new URL('../../plugins/web-admin/assets/', import.meta.url)
await mkdir(target, { recursive: true })
for (const item of await readdir(target)) await rm(new URL(item, target), { recursive: true, force: true })
await cp(new URL('../dist/', import.meta.url), target, { recursive: true })
console.log('Published web/dist → plugins/web-admin/assets')
