import type { SidebarsConfig } from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
    {
      type: 'category',
      label: 'Python',
      link: {
        type: 'doc',
        id: 'python/index',
      },
      items: [
        'python/installation',
        'python/polars',
        'python/pandas',
        'python/pyarrow',
        'python/streaming',
        'python/lazy-scan',
        'python/writing',
        'python/metadata',
        'python/spatial',
        'python/row-reader',
      ],
    },
    {
      type: 'category',
      label: 'Rust',
      link: {
        type: 'doc',
        id: 'rust/index',
      },
      items: [
        'rust/getting-started',
        'rust/reading',
        'rust/writing',
        'rust/field-types',
      ],
    },
    {
      type: 'category',
      label: 'Developer Guide',
      link: {
        type: 'doc',
        id: 'developer/index',
      },
      items: [
        'developer/architecture',
        'developer/building',
        'developer/testing',
        'developer/contributing',
      ],
    },
    'specification',
    'field-type-reference',
  ],
};

export default sidebars;
