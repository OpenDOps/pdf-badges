/** Palette ported from expocab-frontend/apps/visitor-tickets/tailwind.config.js
 * for visual consistency with the rest of expocab — see docs/TASKS_FRONTEND.md
 * задача 2 for why this is a straight palette port rather than importing the
 * Nx component libraries (not distributable outside that monorepo). */
module.exports = {
  content: ['./src/**/*.{html,ts}'],
  theme: {
    screens: { md: '768px', lg: '1024px', xl: '1280px', '2xl': '1680px' },
    colors: {
      'button-primary': '#ed293f',
      'button-primary-hover': '#d62438',
      'button-disabled': '#b1b9ca',
      'button-base': '#ebeef1',
      'button-base-hover': '#e3e7ec',
      'button-secondary': '#f6f7f8',
      'button-secondary-hover': '#eef0f2',
      'button-white': '#fff',
      'text-base': '#20304a',
      'text-base-hover': '#505c71',
      'text-secondary': '#b1b9ca',
      'text-secondary-hover': '#999fae',
      'text-red': '#ed293f',
      'text-green': '#509e52',
      'text-blue': '#296df2',
      'text-white': '#fff',
      'bg-grey': '#f6f7f8',
      'bg-red': '#fdeaec',
      'bg-blue': '#eaf0fe',
      'bg-green': '#e5f1e6',
      'bg-base': '#f6f8fa',
      'bg-white': '#fff',
      'border-white': '#fff',
      'border-grey-light': '#dde2ee',
      'border-grey-medium': '#d1d7de',
      'border-grey-dark': '#bfc4c9',
      'border-black': '#20304a',
      'border-red': '#ed293f',
      'border-green': '#509e52',
      'divider-grey': '#dde2ee',
      'divider-white': '#fff',
      transparent: 'transparent',
      current: 'currentColor',
    },
  },
  corePlugins: {
    preflight: false, // Angular Material already normalizes base styles — avoid fighting it
  },
  plugins: [],
};
