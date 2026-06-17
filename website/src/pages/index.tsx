import React from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import styles from './index.module.css';
import Heading from '@theme/Heading';

const features = [
  {
    title: '7-Layer Memory',
    emoji: '🧠',
    description:
      'Hierarchical memory from agent-level to tenant-wide, giving AI agents persistent context across sessions, teams, and organizations.',
    link: '/docs/specs/memory-system',
  },
  {
    title: 'Knowledge Repository',
    emoji: '📚',
    description:
      'Git-versioned knowledge base for ADRs, patterns, and constraints. Version-controlled organizational learning with full audit trails.',
    link: '/docs/specs/knowledge-repository',
  },
  {
    title: 'Enterprise Governance',
    emoji: '🛡️',
    description:
      'Multi-tenant RBAC with Cedar policies and OPAL integration. Hierarchical tenant isolation with policy inheritance.',
    link: '/docs/governance/policy-model',
  },
  {
    title: 'MCP Tool Interface',
    emoji: '🔌',
    description:
      'Standardized Model Context Protocol with 11 unified tools. Seamless integration with OpenCode, Claude, and any MCP-compatible client.',
    link: '/docs/integrations/mcp-server',
  },
  {
    title: 'Pluggable Adapters',
    emoji: '🔄',
    description:
      'Swap storage backends (Qdrant, Pinecone, Weaviate, MongoDB, Vertex AI, Databricks) and LLM providers without lock-in.',
    link: '/docs/specs/adapter-architecture',
  },
  {
    title: 'Kubernetes Native',
    emoji: '☸️',
    description:
      'Production-ready Helm chart with local, hybrid, and remote deployment modes. CNPG, OPAL, Dragonfly, and full observability stack.',
    link: '/docs/helm/architecture',
  },
];

function HomepageHeader() {
  const { siteConfig } = useDocusaurusContext();
  return (
    <header className={clsx('hero', styles.heroBanner)}>
      <div className="container">
        <Heading as="h1" className={styles.heroTitle}>
          {siteConfig.title}
        </Heading>
        <p className={styles.heroSubtitle}>{siteConfig.tagline}</p>
        <p className={styles.heroDescription}>
          Built for organizations deploying AI coding assistants, autonomous agents,
          and intelligent automation across hundreds of engineers and thousands
          of projects.
        </p>
        <div className={styles.buttons}>
          <Link
            className="button button--primary button--lg"
            to="/docs/">
            Get Started
          </Link>
          <Link
            className="button button--outline button--lg"
            to="https://github.com/kikokikok/aeterna">
            GitHub ↗
          </Link>
        </div>
      </div>
    </header>
  );
}

function FeatureCard({
  title,
  emoji,
  description,
  link,
}: {
  title: string;
  emoji: string;
  description: string;
  link: string;
}) {
  return (
    <div className={clsx('col col--4')}>
      <Link to={link} className={styles.featureCard}>
        <div className={styles.featureEmoji}>{emoji}</div>
        <Heading as="h3" className={styles.featureTitle}>
          {title}
        </Heading>
        <p className={styles.featureDescription}>{description}</p>
      </Link>
    </div>
  );
}

function ArchitectureSection() {
  return (
    <section className={styles.architectureSection}>
      <div className="container">
        <Heading as="h2" className={styles.sectionHeading}>
          Multi-Tenant Hierarchy
        </Heading>
        <p className={styles.sectionDescription}>
          Aeterna's organizational hierarchy enables enterprise-scale deployment
          with policy inheritance flowing from tenant level down to individual projects.
        </p>
        <div className={styles.hierarchyGrid}>
          <div className={styles.hierarchyColumn}>
            <Heading as="h4">Memory Layers (7)</Heading>
            <div className={styles.layerStack}>
              {['Tenant', 'Organization', 'Team', 'Project', 'Session', 'User', 'Agent'].map(
                (layer, i) => (
                  <div
                    key={layer}
                    className={styles.layerItem}
                    style={{ opacity: 1 - i * 0.08 }}>
                    {layer}
                  </div>
                ),
              )}
            </div>
          </div>
          <div className={styles.hierarchyColumn}>
            <Heading as="h4">Knowledge Layers (4)</Heading>
            <div className={styles.layerStack}>
              {['Tenant', 'Organization', 'Team', 'Project'].map(
                (layer, i) => (
                  <div
                    key={layer}
                    className={styles.layerItem}
                    style={{ opacity: 1 - i * 0.1 }}>
                    {layer}
                  </div>
                ),
              )}
            </div>
          </div>
          <div className={styles.hierarchyColumn}>
            <Heading as="h4">RBAC Roles (5)</Heading>
            <div className={styles.layerStack}>
              {['Admin', 'Architect', 'Tech Lead', 'Developer', 'Agent'].map(
                (role, i) => (
                  <div
                    key={role}
                    className={styles.layerItem}
                    style={{ opacity: 1 - i * 0.1 }}>
                    {role}
                  </div>
                ),
              )}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function TechStackSection() {
  return (
    <section className={styles.techSection}>
      <div className="container">
        <Heading as="h2" className={styles.sectionHeading}>
          Built With
        </Heading>
        <div className={styles.techGrid}>
          {[
            { name: 'Rust', detail: 'Edition 2024, Axum HTTP' },
            { name: 'PostgreSQL', detail: '16+ (stock)' },
            { name: 'Cedar', detail: 'Authorization policies' },
            { name: 'OPAL', detail: 'Real-time policy sync' },
            { name: 'Helm', detail: 'K8s deployment' },
            { name: 'MCP', detail: 'Model Context Protocol' },
            { name: 'DuckDB', detail: 'Graph layer' },
            { name: 'Redis', detail: 'Caching & pub/sub' },
          ].map(({ name, detail }) => (
            <div key={name} className={styles.techItem}>
              <strong>{name}</strong>
              <span>{detail}</span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

export default function Home(): JSX.Element {
  const { siteConfig } = useDocusaurusContext();
  return (
    <Layout
      title={siteConfig.title}
      description="Universal Memory & Knowledge Framework for Enterprise AI Agent Systems">
      <HomepageHeader />
      <main>
        <section className={styles.features}>
          <div className="container">
            <Heading as="h2" className={styles.sectionHeading}>
              Core Capabilities
            </Heading>
            <div className="row">
              {features.map((props, idx) => (
                <FeatureCard key={idx} {...props} />
              ))}
            </div>
          </div>
        </section>
        <ArchitectureSection />
        <TechStackSection />
      </main>
    </Layout>
  );
}
