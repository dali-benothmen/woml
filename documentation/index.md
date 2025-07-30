---
layout: home
hero:
  name: 'Cronflow'
  text: 'The Fastest Code-First Workflow Automation Engine'
  tagline: Built with Rust + Bun for unparalleled performance. Replace your entire n8n infrastructure with a single package.
  image:
    src: /code.png
    alt: Cronflow Code
  actions:
    - theme: brand
      text: Get Started
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/dali-benothmen/cronflow
    - theme: alt
      text: API Reference
      link: /api/

features:
  - icon: 🚀
    title: Lightning Fast
    details: 90x faster than traditional tools. Process 10,000+ records in 2ms with true concurrency.
  - icon: 💾
    title: Memory Efficient
    details: Only 0.49MB memory per step vs 500MB+ in traditional tools. 10x less memory consumption.
  - icon: 💰
    title: Zero Cost
    details: Replace $50/month n8n servers with a single npm package. Zero infrastructure costs.
  - icon: 🔧
    title: Code-First
    details: Version control your workflows with Git. Express complex logic with TypeScript and Rust.
  - icon: ⚡
    title: Production Ready
    details: Built with Rust for reliability and Bun for speed. Ready for production in 30 seconds.
  - icon: 🎯
    title: Developer Friendly
    details: Full TypeScript support, comprehensive API, and extensive examples.
---

<style>
/* Override VitePress default colors with green theme */
:root {
  --vp-c-brand: #22c55e;
  --vp-c-brand-light: #4ade80;
  --vp-c-brand-lighter: #86efac;
  --vp-c-brand-dark: #16a34a;
  --vp-c-brand-darker: #15803d;
  --vp-c-brand-highlight: #22c55e;
  --vp-c-brand-hover: #4ade80;
  --vp-c-brand-active: #16a34a;
  
  /* Override indigo colors with green variations */
  --vp-c-indigo-1: #86efac;
  --vp-c-indigo-2: #4ade80;
  --vp-c-indigo-3: #22c55e;
  
  /* Override purple colors with green variations */
  --vp-c-purple-1: #bbf7d0;
  --vp-c-purple-2: #86efac;
  --vp-c-purple-3: #4ade80;
}

/* Override button colors specifically */
.VPButton.brand {
  background-color: #22c55e !important;
  border-color: #22c55e !important;
}

.VPButton.brand:hover {
  background-color: #4ade80 !important;
  border-color: #4ade80 !important;
}

.VPButton.alt {
  border-color: #22c55e !important;
  color: #22c55e !important;
}

.VPButton.alt:hover {
  background-color: #22c55e !important;
  color: white !important;
}

/* Override any remaining blue colors */
.VPHomeHero .actions .VPButton.brand {
  background-color: #22c55e !important;
  border-color: #22c55e !important;
}

.VPHomeHero .actions .VPButton.brand:hover {
  background-color: #4ade80 !important;
  border-color: #4ade80 !important;
}

/* Ensure logo stays in proper position */
.VPNav .logo {
  position: relative !important;
  top: auto !important;
  bottom: auto !important;
  left: auto !important;
  right: auto !important;
  transform: none !important;
}

/* Hero background effects */
:root {
  --vp-home-hero-image-background-image: linear-gradient(
    -45deg,
    rgba(34, 197, 94, 0.1) 0%,
    rgba(34, 197, 94, 0.05) 25%,
    rgba(34, 197, 94, 0.1) 50%,
    rgba(34, 197, 94, 0.05) 75%,
    rgba(34, 197, 94, 0.1) 100%
  );
  --vp-home-hero-image-filter: blur(120px);
}

.VPHomeHero .image {
  position: relative;
}

.VPHomeHero .image::before {
  content: '';
  position: absolute;
  top: -50%;
  left: -50%;
  width: 200%;
  height: 200%;
  background: radial-gradient(
    circle at center,
    rgba(34, 197, 94, 0.15) 0%,
    rgba(34, 197, 94, 0.1) 30%,
    rgba(34, 197, 94, 0.05) 60%,
    transparent 100%
  );
  border-radius: 50%;
  filter: blur(80px);
  z-index: -1;
}

.VPHomeHero .image::after {
  content: '';
  position: absolute;
  top: 20%;
  right: -30%;
  width: 60%;
  height: 60%;
  background: linear-gradient(
    45deg,
    rgba(34, 197, 94, 0.1) 0%,
    rgba(34, 197, 94, 0.05) 50%,
    transparent 100%
  );
  border-radius: 30% 70% 70% 30% / 30% 30% 70% 70%;
  filter: blur(60px);
  z-index: -1;
}

/* Enhanced white box container for the image with smooth animation */
.VPHomeHero .image img {
  z-index: 1;
  background: white;
  border-radius: 20px;
  transform: rotate(5deg);
  top: 15%;
  left: 45%;
  box-shadow: 
    0 20px 60px rgba(0, 0, 0, 0.15),
    0 8px 32px rgba(0, 0, 0, 0.1),
    0 0 40px rgba(34, 197, 94, 0.3),
    0 0 80px rgba(34, 197, 94, 0.2);
  padding: 20px;
  border: 3px solid #22c55e;
  animation: float 3s ease-in-out infinite;
}

/* Smooth up and down floating animation */
@keyframes float {
  0%, 100% {
    transform: rotate(5deg) translateY(0px);
  }
  50% {
    transform: rotate(5deg) translateY(-10px);
  }
}
</style>
