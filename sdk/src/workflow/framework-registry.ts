export type SupportedFramework =
  | 'express'
  | 'fastify'
  | 'koa'
  | 'hapi'
  | 'nestjs'
  | 'bun'
  | 'nextjs';

export interface FrameworkRegistry {
  [key: string]: (
    app: any,
    method: string,
    path: string,
    handler: Function
  ) => void;
}

export const frameworkRegistry: FrameworkRegistry = {
  express: (app: any, method: string, path: string, handler: Function) => {
    app[method.toLowerCase()](path, handler);
  },
  fastify: (app: any, method: string, path: string, handler: Function) => {
    app[method.toLowerCase()](path, handler);
  },
  koa: (app: any, method: string, path: string, handler: Function) => {
    if (app.router) {
      app.router[method.toLowerCase()](path, handler);
    } else {
      app[method.toLowerCase()](path, handler);
    }
  },
  hapi: (app: any, method: string, path: string, handler: Function) => {
    app.route({
      method: method.toLowerCase(),
      path,
      handler: async (request: any, h: any) => {
        const req = {
          headers: request.headers,
          body: request.payload,
          method: request.method,
          url: request.url,
        };
        const res = {
          status: (code: number) => ({
            json: (data: any) => h.response(data).code(code),
          }),
        };
        return handler(req, res);
      },
    });
  },
  nestjs: (app: any, method: string, path: string, handler: Function) => {
    if (app.use) {
      app[method.toLowerCase()](path, handler);
    } else {
      console.warn('⚠️ NestJS integration requires Express app instance');
    }
  },
  bun: (app: any, method: string, path: string, handler: Function) => {
    if (typeof (globalThis as any).Bun !== 'undefined') {
      (globalThis as any).Bun.serve({
        port: 3000,
        fetch: (req: Request) => {
          const url = new URL(req.url);
          if (req.method === method.toUpperCase() && url.pathname === path) {
            const standardReq = {
              headers: Object.fromEntries((req.headers as any).entries()),
              body: req.body,
              method: req.method,
              url: req.url,
            };
            const standardRes = {
              status: (code: number) => ({
                json: (data: any) =>
                  new Response(JSON.stringify(data), {
                    status: code,
                    headers: { 'Content-Type': 'application/json' },
                  }),
              }),
            };
            return handler(standardReq, standardRes);
          }
          return new Response('Not Found', { status: 404 });
        },
      });
    } else {
      console.warn('⚠️ Bun runtime not detected');
    }
  },
  nextjs: (app: any, method: string, path: string, handler: Function) => {
    if (app.use) {
      app[method.toLowerCase()](path, handler);
    } else {
      console.warn(
        '⚠️ Next.js integration requires Express app instance or file-based API routes'
      );
    }
  },
};

export function getFrameworkHandler(
  framework: SupportedFramework
): (app: any, method: string, path: string, handler: Function) => void {
  const handler = frameworkRegistry[framework];
  if (!handler) {
    throw new Error(
      `Unsupported framework: ${framework}. Supported frameworks: ${Object.keys(frameworkRegistry).join(', ')}`
    );
  }
  return handler;
}

export function isFrameworkSupported(
  framework: string
): framework is SupportedFramework {
  return framework in frameworkRegistry;
}

export function getSupportedFrameworks(): SupportedFramework[] {
  return Object.keys(frameworkRegistry) as SupportedFramework[];
}
