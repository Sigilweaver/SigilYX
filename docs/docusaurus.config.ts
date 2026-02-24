import { themes as prismThemes } from 'prism-react-renderer';
import type { Config } from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'SigilYX',
  tagline: 'High-performance YXDB reader and writer in Rust, with Python bindings',
  favicon: 'img/favicon.ico',

  future: {
    v4: true,
  },

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },
  themes: ['@docusaurus/theme-mermaid'],

  // Set the production url of your site here
  url: 'https://sigilweaver.app',
  // Serve in /sigilyx subfolder
  baseUrl: '/sigilyx/',

  organizationName: 'sigilweaver',
  projectName: 'sigilyx',

  onBrokenLinks: 'throw',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          routeBasePath: '/',
          sidebarPath: './sidebars.ts',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    metadata: [
      { name: 'description', content: 'SigilYX — high-performance YXDB reader and writer in Rust with Python bindings for Polars, PyArrow, and Pandas.' },
      { name: 'keywords', content: 'yxdb, sigilyx, alteryx, polars, python, rust, sigilweaver' },
    ],
    colorMode: {
      defaultMode: 'dark',
      disableSwitch: false,
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'SigilYX',
      logo: {
        alt: 'Sigilweaver Logo',
        src: 'img/logo.svg',
        href: 'https://sigilweaver.app/sigilyx',
        target: '_self',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Documentation',
        },
        {
          href: 'https://sigilweaver.app',
          label: 'Sigilweaver',
          position: 'right',
        },
        {
          href: 'https://sigilweaver.app/docs',
          label: 'Sigilweaver Docs',
          position: 'right',
        },
        {
          href: 'https://github.com/sigilweaver/sigilyx',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Documentation',
          items: [
            {
              label: 'Getting Started',
              to: '/',
            },
            {
              label: 'Python Guide',
              to: '/python',
            },
            {
              label: 'Rust Guide',
              to: '/rust/getting-started',
            },
          ],
        },
        {
          title: 'Sigilweaver',
          items: [
            {
              label: 'Website',
              href: 'https://sigilweaver.app',
            },
            {
              label: 'Documentation',
              href: 'https://sigilweaver.app/docs',
            },
            {
              label: 'Downloads',
              href: 'https://sigilweaver.app/downloads',
            },
          ],
        },
        {
          title: 'Community',
          items: [
            {
              label: 'GitHub',
              href: 'https://github.com/sigilweaver/sigilyx',
            },
            {
              label: 'GitHub Discussions',
              href: 'https://github.com/sigilweaver/sigilweaver/discussions',
            },
          ],
        },
        {
          title: 'Legal',
          items: [
            {
              label: 'License (AGPL-3.0)',
              href: 'https://github.com/sigilweaver/sigilyx/blob/main/LICENSE',
            },
            {
              label: 'Terms of Use',
              href: 'https://sigilweaver.app/terms',
            },
            {
              label: 'Privacy Policy',
              href: 'https://sigilweaver.app/privacy',
            },
          ],
        },
      ],
      copyright: `© ${new Date().getFullYear()} Sigilweaver Holdings LLC. Sigilweaver™ is a trademark of Sigilweaver Holdings LLC. Documentation licensed under <a href="https://creativecommons.org/licenses/by-sa/4.0/" target="_blank" rel="noopener noreferrer">CC-BY-SA 4.0</a>. Operated under license by Sigilweaver LLC.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['python', 'bash', 'json', 'rust', 'toml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
