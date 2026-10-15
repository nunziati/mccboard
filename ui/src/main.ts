import { mount } from 'svelte'
import App from './App.svelte'
import './app.css'
import { i18n } from './lib/i18n.svelte'

document.documentElement.lang = i18n.lang

const app = mount(App, { target: document.getElementById('app')! })

export default app
