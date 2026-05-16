import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  integrations: [
    starlight({
      title: 'sivtr',
      description:
        'Documentation for sivtr, a terminal output workspace for capturing, browsing, searching, selecting, and reusing command output.',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/Ariestar/sivtr',
        },
      ],
      locales: {
        root: { label: 'English', lang: 'en' },
        'zh-cn': { label: '简体中文', lang: 'zh-CN' },
      },
      defaultLocale: 'root',
      favicon: '/favicon.svg',
      customCss: ['./src/styles/custom.css'],
      tableOfContents: {
        minHeadingLevel: 2,
        maxHeadingLevel: 3,
      },
      lastUpdated: true,
      sidebar: [
        {
          label: 'Overview',
          translations: { 'zh-CN': '概览' },
          link: '/',
        },
        {
          label: 'Start',
          translations: { 'zh-CN': '开始' },
          items: [
            'start/installation',
            'start/quickstart',
            'start/core-concepts',
          ],
        },
        {
          label: 'Use sivtr',
          translations: { 'zh-CN': '使用 sivtr' },
          items: [
            'usage/capture-output',
            'usage/browse-and-select',
            'usage/copy-command-blocks',
            'usage/codex-capture',
            'usage/history',
            'usage/configuration',
            'usage/hotkey',
          ],
        },
        {
          label: 'Reference',
          translations: { 'zh-CN': '参考' },
          items: [
            'reference/cli',
            'reference/keybindings',
            'reference/config-file',
          ],
        },
        {
          label: 'Explanation',
          translations: { 'zh-CN': '解释' },
          items: [
            'explanation/architecture',
            'explanation/session-model',
          ],
        },
        {
          label: 'Project',
          translations: { 'zh-CN': '项目' },
          items: ['project/roadmap', 'project/release-notes'],
        },
      ],
    }),
  ],
});
