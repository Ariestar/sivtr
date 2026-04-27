import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  integrations: [
    starlight({
      title: 'sivtr',
      description:
        'Documentation for sivtr, a terminal output workspace for capturing, browsing, searching, selecting, and reusing command output.',
      favicon: '/favicon.svg',
      customCss: ['./src/styles/custom.css'],
      tableOfContents: {
        minHeadingLevel: 2,
        maxHeadingLevel: 3,
      },
      lastUpdated: true,
      sidebar: [
        { label: 'Overview', link: '/' },
        {
          label: 'Start',
          items: [
            'start/installation',
            'start/quickstart',
            'start/core-concepts',
          ],
        },
        {
          label: 'Use sivtr',
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
          items: [
            'reference/cli',
            'reference/keybindings',
            'reference/config-file',
          ],
        },
        {
          label: 'Explanation',
          items: [
            'explanation/architecture',
            'explanation/session-model',
          ],
        },
        {
          label: 'Project',
          items: ['project/release-notes', 'project/documentation-maintenance'],
        },
      ],
    }),
  ],
});
