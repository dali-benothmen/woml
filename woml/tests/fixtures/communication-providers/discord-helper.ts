declare const services: {
  readonly discord: {
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
  return services.discord.send(
    { botToken, conversationId, text },
    { name: 'discord-helper-reply' }
  );
}
