declare const services: {
  readonly telegram: {
    readonly send: (
      input: Readonly<{
        botToken: string;
        conversationId: string;
        text: string;
      }>,
      options?: Readonly<{ name?: string }>
    ) => Promise<unknown>;
  };
};

export async function sendReply(
  botToken: string,
  conversationId: string,
  text: string
) {
  return services.telegram.send(
    { botToken, conversationId, text },
    { name: 'telegram-helper-reply' }
  );
}
