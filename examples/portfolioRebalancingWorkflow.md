# Intelligent Portfolio Rebalancing System

AI-powered portfolio analysis with tax-efficient rebalancing and risk management

```typescript
import { cronflow } from 'cronflow';
import express from 'express';
import { z } from 'zod';

const app = express();
app.use(express.json());

// Types for portfolio management
interface Holding {
  symbol: string;
  name: string;
  shares: number;
  currentPrice: number;
  marketValue: number;
  costBasis: number;
  unrealizedGainLoss: number;
  unrealizedGainLossPercent: number;
  sector: string;
  assetClass: 'equity' | 'bond' | 'commodity' | 'reit' | 'crypto' | 'cash';
  beta?: number;
  dividend?: number;
  expenseRatio?: number;
}

interface Portfolio {
  id: string;
  accountId: string;
  totalValue: number;
  targetAllocation: Record<string, number>; // asset class -> percentage
  currentAllocation: Record<string, number>;
  holdings: Holding[];
  riskTolerance: 'conservative' | 'moderate' | 'aggressive';
  taxableAccount: boolean;
  lastRebalanced: string;
  rebalanceThreshold: number; // deviation percentage to trigger rebalancing
}

interface MarketData {
  symbol: string;
  price: number;
  change: number;
  changePercent: number;
  volume: number;
  marketCap?: number;
  pe?: number;
  dividend?: number;
  beta?: number;
  volatility?: number;
}

interface RebalanceAction {
  symbol: string;
  action: 'buy' | 'sell';
  shares: number;
  estimatedValue: number;
  reason: string;
  taxImplication?: {
    gainLoss: number;
    taxLiability: number;
    holdingPeriod: 'short' | 'long';
  };
}

interface TaxOptimization {
  harvestableGains: number;
  harvestableLosses: number;
  taxSavings: number;
  recommendations: string[];
  washSaleRisk: boolean;
}

// Mock financial data services (replace with real APIs like Alpha Vantage, Yahoo Finance, etc.)
async function getMarketData(
  symbols: string[]
): Promise<Record<string, MarketData>> {
  console.log('📊 Fetching real-time market data for:', symbols);

  // Simulate market data with realistic variations
  const marketData: Record<string, MarketData> = {};

  for (const symbol of symbols) {
    const basePrice = Math.random() * 200 + 50; // $50-$250 range
    const changePercent = (Math.random() - 0.5) * 10; // -5% to +5%

    marketData[symbol] = {
      symbol,
      price: basePrice,
      change: basePrice * (changePercent / 100),
      changePercent,
      volume: Math.floor(Math.random() * 1000000),
      marketCap: Math.floor(Math.random() * 100) * 1e9,
      pe: Math.random() * 30 + 5,
      dividend: Math.random() * 5,
      beta: Math.random() * 2 + 0.5,
      volatility: Math.random() * 0.3 + 0.1,
    };
  }

  return marketData;
}

async function getPortfolioData(accountId: string): Promise<Portfolio> {
  console.log('📈 Fetching portfolio data for account:', accountId);

  // Mock portfolio data
  const holdings: Holding[] = [
    {
      symbol: 'VTI',
      name: 'Vanguard Total Stock Market ETF',
      shares: 100,
      currentPrice: 220.5,
      marketValue: 22050,
      costBasis: 200,
      unrealizedGainLoss: 2050,
      unrealizedGainLossPercent: 10.25,
      sector: 'Broad Market',
      assetClass: 'equity',
      beta: 1.0,
      dividend: 1.8,
      expenseRatio: 0.03,
    },
    {
      symbol: 'BND',
      name: 'Vanguard Total Bond Market ETF',
      shares: 200,
      currentPrice: 85.3,
      marketValue: 17060,
      costBasis: 90,
      unrealizedGainLoss: -940,
      unrealizedGainLossPercent: -5.22,
      sector: 'Government/Corporate Bonds',
      assetClass: 'bond',
      beta: 0.1,
      dividend: 2.4,
      expenseRatio: 0.05,
    },
    {
      symbol: 'VEA',
      name: 'Vanguard FTSE Developed Markets ETF',
      shares: 150,
      currentPrice: 48.9,
      marketValue: 7335,
      costBasis: 50,
      unrealizedGainLoss: -165,
      unrealizedGainLossPercent: -2.2,
      sector: 'International Developed',
      assetClass: 'equity',
      beta: 0.8,
      dividend: 2.1,
      expenseRatio: 0.05,
    },
    {
      symbol: 'VNQ',
      name: 'Vanguard Real Estate Investment Trust ETF',
      shares: 50,
      currentPrice: 95.2,
      marketValue: 4760,
      costBasis: 100,
      unrealizedGainLoss: -240,
      unrealizedGainLossPercent: -4.8,
      sector: 'Real Estate',
      assetClass: 'reit',
      beta: 0.9,
      dividend: 3.2,
      expenseRatio: 0.12,
    },
  ];

  const totalValue = holdings.reduce((sum, h) => sum + h.marketValue, 0);

  return {
    id: `portfolio_${accountId}`,
    accountId,
    totalValue,
    targetAllocation: {
      equity: 70,
      bond: 20,
      reit: 10,
      cash: 0,
    },
    currentAllocation: {
      equity: 57.8, // (22050 + 7335) / 51205
      bond: 33.3, // 17060 / 51205
      reit: 9.3, // 4760 / 51205
      cash: 0,
    },
    holdings,
    riskTolerance: 'moderate',
    taxableAccount: true,
    lastRebalanced: '2024-01-01T00:00:00Z',
    rebalanceThreshold: 5.0, // 5% deviation threshold
  };
}

async function calculateRebalancing(
  portfolio: Portfolio,
  marketData: Record<string, MarketData>
): Promise<{
  needsRebalancing: boolean;
  deviations: Record<string, number>;
  actions: RebalanceAction[];
  estimatedCosts: number;
}> {
  console.log('⚖️ Calculating portfolio rebalancing requirements...');

  const deviations: Record<string, number> = {};
  let needsRebalancing = false;

  // Calculate current vs target allocation deviations
  for (const [assetClass, targetPercent] of Object.entries(
    portfolio.targetAllocation
  )) {
    const currentPercent = portfolio.currentAllocation[assetClass] || 0;
    const deviation = Math.abs(currentPercent - targetPercent);
    deviations[assetClass] = deviation;

    if (deviation > portfolio.rebalanceThreshold) {
      needsRebalancing = true;
    }
  }

  const actions: RebalanceAction[] = [];
  let estimatedCosts = 0;

  if (needsRebalancing) {
    // Calculate required trades to reach target allocation
    for (const [assetClass, targetPercent] of Object.entries(
      portfolio.targetAllocation
    )) {
      const currentPercent = portfolio.currentAllocation[assetClass] || 0;
      const targetValue = portfolio.totalValue * (targetPercent / 100);
      const currentValue = portfolio.totalValue * (currentPercent / 100);
      const difference = targetValue - currentValue;

      if (Math.abs(difference) > 100) {
        // Only trade if difference > $100
        const holdings = portfolio.holdings.filter(
          h => h.assetClass === assetClass
        );

        if (holdings.length > 0) {
          const primaryHolding = holdings[0]; // Use largest holding for the asset class
          const currentPrice =
            marketData[primaryHolding.symbol]?.price ||
            primaryHolding.currentPrice;
          const shares = Math.abs(Math.floor(difference / currentPrice));

          if (shares > 0) {
            actions.push({
              symbol: primaryHolding.symbol,
              action: difference > 0 ? 'buy' : 'sell',
              shares,
              estimatedValue: shares * currentPrice,
              reason: `Rebalance ${assetClass} from ${currentPercent.toFixed(1)}% to ${targetPercent}%`,
              taxImplication:
                portfolio.taxableAccount && difference < 0
                  ? {
                      gainLoss:
                        shares * (currentPrice - primaryHolding.costBasis),
                      taxLiability:
                        shares *
                        (currentPrice - primaryHolding.costBasis) *
                        0.2, // 20% capital gains
                      holdingPeriod: 'long', // Assume long-term holdings
                    }
                  : undefined,
            });

            estimatedCosts += shares * currentPrice * 0.001; // 0.1% trading cost estimate
          }
        }
      }
    }
  }

  return { needsRebalancing, deviations, actions, estimatedCosts };
}

async function performTaxLossHarvesting(
  portfolio: Portfolio
): Promise<TaxOptimization> {
  console.log('🏛️ Analyzing tax-loss harvesting opportunities...');

  let harvestableGains = 0;
  let harvestableLosses = 0;
  const recommendations: string[] = [];

  for (const holding of portfolio.holdings) {
    if (holding.unrealizedGainLoss < -500) {
      // Losses > $500
      harvestableLosses += Math.abs(holding.unrealizedGainLoss);
      recommendations.push(
        `Consider harvesting loss in ${holding.symbol}: $${Math.abs(holding.unrealizedGainLoss).toFixed(2)}`
      );
    } else if (holding.unrealizedGainLoss > 1000) {
      // Gains > $1000
      harvestableGains += holding.unrealizedGainLoss;
    }
  }

  const taxSavings = harvestableLosses * 0.22; // 22% tax bracket assumption
  const washSaleRisk = harvestableLosses > 0; // Risk if buying similar securities within 30 days

  if (harvestableLosses > 1000) {
    recommendations.push(
      `Potential tax savings: $${taxSavings.toFixed(2)} from loss harvesting`
    );
  }

  if (washSaleRisk) {
    recommendations.push(
      '⚠️ Avoid wash sale rule: Wait 31 days before repurchasing similar securities'
    );
  }

  return {
    harvestableGains,
    harvestableLosses,
    taxSavings,
    recommendations,
    washSaleRisk,
  };
}

async function analyzeMarketConditions(): Promise<{
  marketSentiment: 'bullish' | 'bearish' | 'neutral';
  volatilityIndex: number;
  riskRecommendation: string;
  marketTiming: 'good' | 'poor' | 'neutral';
}> {
  console.log('📊 Analyzing current market conditions...');

  // Simulate market analysis
  const volatilityIndex = Math.random() * 40 + 10; // 10-50 VIX range
  const sentimentScore = Math.random() * 2 - 1; // -1 to 1

  const marketSentiment =
    sentimentScore > 0.3
      ? 'bullish'
      : sentimentScore < -0.3
        ? 'bearish'
        : 'neutral';

  const marketTiming =
    volatilityIndex > 30 ? 'poor' : volatilityIndex < 20 ? 'good' : 'neutral';

  let riskRecommendation = '';
  if (volatilityIndex > 30) {
    riskRecommendation =
      'High volatility detected. Consider defensive positioning or delayed rebalancing.';
  } else if (marketSentiment === 'bearish') {
    riskRecommendation =
      'Bearish sentiment. Consider dollar-cost averaging into positions.';
  } else {
    riskRecommendation = 'Market conditions favorable for rebalancing.';
  }

  return {
    marketSentiment,
    volatilityIndex,
    riskRecommendation,
    marketTiming,
  };
}

async function executeTrades(actions: RebalanceAction[]): Promise<{
  executedTrades: Array<{
    symbol: string;
    action: string;
    shares: number;
    executionPrice: number;
    timestamp: string;
    orderId: string;
  }>;
  totalCost: number;
  errors: string[];
}> {
  console.log('💼 Executing rebalancing trades...');

  const executedTrades: Array<{
    symbol: string;
    action: string;
    shares: number;
    executionPrice: number;
    timestamp: string;
    orderId: string;
  }> = [];
  let totalCost = 0;
  const errors: string[] = [];

  for (const action of actions) {
    try {
      // Simulate trade execution with slight price slippage
      const slippage = (Math.random() - 0.5) * 0.02; // ±1% slippage
      const executionPrice =
        (action.estimatedValue / action.shares) * (1 + slippage);

      executedTrades.push({
        symbol: action.symbol,
        action: action.action,
        shares: action.shares,
        executionPrice,
        timestamp: new Date().toISOString(),
        orderId: `order_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`,
      });

      totalCost += action.shares * executionPrice;

      console.log(
        `✅ ${action.action.toUpperCase()} ${action.shares} shares of ${action.symbol} at $${executionPrice.toFixed(2)}`
      );
    } catch (error) {
      errors.push(
        `Failed to execute ${action.action} for ${action.symbol}: ${error}`
      );
    }
  }

  return { executedTrades, totalCost, errors };
}

// Portfolio rebalancing workflow schema
const rebalanceRequestSchema = z.object({
  accountId: z.string().min(1),
  forceRebalance: z.boolean().default(false),
  maxTradeAmount: z.number().positive().optional(),
  excludeSymbols: z.array(z.string()).default([]),
  dryRun: z.boolean().default(false),
  taxOptimization: z.boolean().default(true),
  riskOverride: z.enum(['conservative', 'moderate', 'aggressive']).optional(),
});

// Define the portfolio rebalancing workflow
const portfolioRebalancingWorkflow = cronflow.define({
  id: 'portfolio-rebalancing-agent',
  name: 'Intelligent Portfolio Rebalancing System',
  description:
    'AI-powered portfolio analysis with tax-efficient rebalancing and risk management',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        console.log('🎉 Portfolio rebalancing completed successfully!');
        console.log('📊 Final results:', JSON.stringify(ctx.last, null, 2));
      }
    },
    onFailure: (ctx, stepId) => {
      console.log(`❌ Portfolio rebalancing failed at step: ${stepId}`);
      console.log('Error:', ctx.step_error || ctx.error);
    },
  },
});

portfolioRebalancingWorkflow
  // Step 1: Load portfolio data and validate account
  .step('load-portfolio', async ctx => {
    console.log('📋 Loading portfolio data...');

    const portfolio = await getPortfolioData(ctx.payload.accountId);

    // Apply risk override if specified
    if (ctx.payload.riskOverride) {
      portfolio.riskTolerance = ctx.payload.riskOverride;
    }

    // Adjust target allocation based on risk tolerance
    if (portfolio.riskTolerance === 'conservative') {
      portfolio.targetAllocation = { equity: 40, bond: 50, reit: 5, cash: 5 };
    } else if (portfolio.riskTolerance === 'aggressive') {
      portfolio.targetAllocation = { equity: 85, bond: 10, reit: 5, cash: 0 };
    }

    console.log(
      `💰 Portfolio value: $${portfolio.totalValue.toLocaleString()}`
    );
    console.log(`⚖️ Risk tolerance: ${portfolio.riskTolerance}`);

    return { portfolio, accountId: ctx.payload.accountId };
  })

  // Step 2: Fetch real-time market data
  .step('fetch-market-data', async ctx => {
    console.log('📈 Fetching real-time market data...');

    const symbols = ctx.last.portfolio.holdings.map(h => h.symbol);
    const marketData = await getMarketData(symbols);

    // Update portfolio with current market prices
    const updatedHoldings = ctx.last.portfolio.holdings.map(holding => {
      const currentData = marketData[holding.symbol];
      if (currentData) {
        return {
          ...holding,
          currentPrice: currentData.price,
          marketValue: holding.shares * currentData.price,
          unrealizedGainLoss:
            holding.shares * (currentData.price - holding.costBasis),
          unrealizedGainLossPercent:
            ((currentData.price - holding.costBasis) / holding.costBasis) * 100,
        };
      }
      return holding;
    });

    const updatedPortfolio = {
      ...ctx.last.portfolio,
      holdings: updatedHoldings,
      totalValue: updatedHoldings.reduce((sum, h) => sum + h.marketValue, 0),
    };

    // Recalculate current allocation
    const currentAllocation: Record<string, number> = {};
    for (const assetClass of Object.keys(updatedPortfolio.targetAllocation)) {
      const classValue = updatedHoldings
        .filter(h => h.assetClass === assetClass)
        .reduce((sum, h) => sum + h.marketValue, 0);
      currentAllocation[assetClass] =
        (classValue / updatedPortfolio.totalValue) * 100;
    }
    updatedPortfolio.currentAllocation = currentAllocation;

    return {
      portfolio: updatedPortfolio,
      marketData,
      accountId: ctx.last.accountId,
    };
  })

  // Step 3: Analyze market conditions
  .step('analyze-market-conditions', async ctx => {
    console.log('🌍 Analyzing market conditions...');

    const marketAnalysis = await analyzeMarketConditions();

    console.log(`📊 Market sentiment: ${marketAnalysis.marketSentiment}`);
    console.log(
      `📈 Volatility index: ${marketAnalysis.volatilityIndex.toFixed(1)}`
    );
    console.log(`💡 ${marketAnalysis.riskRecommendation}`);

    return {
      portfolio: ctx.last.portfolio,
      marketData: ctx.last.marketData,
      marketAnalysis,
      accountId: ctx.last.accountId,
    };
  })

  // Step 4: Calculate rebalancing requirements
  .step('calculate-rebalancing', async ctx => {
    console.log('⚖️ Calculating rebalancing requirements...');

    const rebalanceAnalysis = await calculateRebalancing(
      ctx.last.portfolio,
      ctx.last.marketData
    );

    console.log(`🎯 Needs rebalancing: ${rebalanceAnalysis.needsRebalancing}`);
    if (rebalanceAnalysis.needsRebalancing) {
      console.log('📊 Asset class deviations:');
      Object.entries(rebalanceAnalysis.deviations).forEach(
        ([asset, deviation]) => {
          if (deviation > ctx.last.portfolio.rebalanceThreshold) {
            console.log(
              `  - ${asset}: ${deviation.toFixed(1)}% (threshold: ${ctx.last.portfolio.rebalanceThreshold}%)`
            );
          }
        }
      );
      console.log(
        `💵 Estimated trading costs: $${rebalanceAnalysis.estimatedCosts.toFixed(2)}`
      );
    }

    return {
      portfolio: ctx.last.portfolio,
      marketData: ctx.last.marketData,
      marketAnalysis: ctx.last.marketAnalysis,
      rebalanceAnalysis,
      accountId: ctx.last.accountId,
    };
  })

  // Step 5: Tax optimization analysis (for taxable accounts)
  .step('tax-optimization', async ctx => {
    console.log('🏛️ Performing tax optimization analysis...');

    let taxOptimization: TaxOptimization | null = null;

    if (ctx.last.portfolio.taxableAccount && ctx.payload.taxOptimization) {
      taxOptimization = await performTaxLossHarvesting(ctx.last.portfolio);

      if (taxOptimization) {
        console.log(
          `💰 Harvestable losses: $${taxOptimization.harvestableLosses.toFixed(2)}`
        );
        console.log(
          `💸 Potential tax savings: $${taxOptimization.taxSavings.toFixed(2)}`
        );

        if (taxOptimization.recommendations.length > 0) {
          console.log('💡 Tax optimization recommendations:');
          taxOptimization.recommendations.forEach(rec =>
            console.log(`  - ${rec}`)
          );
        }
      }
    }

    return {
      portfolio: ctx.last.portfolio,
      marketData: ctx.last.marketData,
      marketAnalysis: ctx.last.marketAnalysis,
      rebalanceAnalysis: ctx.last.rebalanceAnalysis,
      taxOptimization,
      accountId: ctx.last.accountId,
    };
  })

  // Conditional: Execute rebalancing if needed and market conditions are favorable
  .if('should-rebalance', ctx => {
    const shouldRebalance =
      ctx.payload.forceRebalance ||
      (ctx.last.rebalanceAnalysis.needsRebalancing &&
        ctx.last.marketAnalysis.marketTiming !== 'poor');

    console.log(`🤔 Should rebalance: ${shouldRebalance}`);
    return shouldRebalance;
  })

  // Step 6: Execute trades (if not dry run)
  .step('execute-trades', async ctx => {
    if (ctx.payload.dryRun) {
      console.log('🧪 DRY RUN: Simulating trade execution...');
      return {
        executedTrades: [],
        totalCost: 0,
        errors: [],
        dryRun: true,
        simulatedActions: ctx.last.rebalanceAnalysis.actions,
      };
    }

    console.log('💼 Executing rebalancing trades...');
    const tradeResults = await executeTrades(
      ctx.last.rebalanceAnalysis.actions
    );

    if (tradeResults.errors.length > 0) {
      console.log('⚠️ Trade execution errors:');
      tradeResults.errors.forEach(error => console.log(`  - ${error}`));
    }

    return {
      executedTrades: tradeResults.executedTrades,
      totalCost: tradeResults.totalCost,
      errors: tradeResults.errors,
      dryRun: false,
    };
  })

  // Step 7: Update portfolio records
  .step('update-portfolio', async ctx => {
    console.log('📝 Updating portfolio records...');

    // Simulate database update with new portfolio state
    const updatedPortfolio = {
      ...ctx.last.portfolio,
      lastRebalanced: new Date().toISOString(),
    };

    console.log('✅ Portfolio records updated successfully');

    return {
      updatedPortfolio,
      rebalanceCompleted: !ctx.last.dryRun,
    };
  })

  .endIf()

  // Step 8: Generate comprehensive report
  .step('generate-report', async ctx => {
    console.log('📊 Generating portfolio analysis report...');

    const report = {
      accountId: ctx.last.accountId,
      portfolioValue: ctx.last.portfolio.totalValue,
      analysis: {
        needsRebalancing: ctx.last.rebalanceAnalysis.needsRebalancing,
        deviations: ctx.last.rebalanceAnalysis.deviations,
        marketConditions: ctx.last.marketAnalysis,
        taxOptimization: ctx.last.taxOptimization,
      },
      actions: ctx.last.rebalanceAnalysis.actions,
      execution: {
        executed:
          !ctx.payload.dryRun && ctx.last.rebalanceAnalysis.needsRebalancing,
        dryRun: ctx.payload.dryRun,
        trades: ctx.last.executedTrades || [],
        totalCost: ctx.last.totalCost || 0,
        errors: ctx.last.errors || [],
      },
      recommendations: [
        ...(ctx.last.taxOptimization?.recommendations || []),
        ctx.last.marketAnalysis.riskRecommendation,
      ].filter(Boolean),
      timestamp: new Date().toISOString(),
    };

    return { report, success: true };
  })

  // Background notifications and logging
  .action('send-notifications', async ctx => {
    console.log('📧 Sending portfolio rebalancing notifications...');

    // Simulate sending email/SMS notifications to client
    const report = ctx.last.report;
    const message =
      `Portfolio rebalancing ${report.execution.executed ? 'completed' : 'analyzed'} for account ${report.accountId}. ` +
      `Portfolio value: $${report.portfolioValue.toLocaleString()}. ` +
      `${report.execution.trades.length} trades executed.`;

    console.log('📱 Notification sent:', message);
  });

// Route to trigger portfolio rebalancing
app.post('/api/portfolio/rebalance', async (req, res) => {
  try {
    console.log('🚀 Triggering portfolio rebalancing workflow...');

    const validatedRequest = rebalanceRequestSchema.parse(req.body);

    const runId = await cronflow.trigger(
      'portfolio-rebalancing-agent',
      validatedRequest
    );

    res.json({
      success: true,
      message: 'Portfolio rebalancing workflow initiated',
      runId,
      accountId: validatedRequest.accountId,
      dryRun: validatedRequest.dryRun,
      timestamp: new Date().toISOString(),
    });
  } catch (error) {
    console.error('Portfolio rebalancing trigger error:', error);
    res.status(400).json({
      success: false,
      error:
        (error as Error).message || 'Failed to trigger portfolio rebalancing',
    });
  }
});

// Route to get portfolio status
app.get('/api/portfolio/:accountId/status', async (req, res) => {
  try {
    const { accountId } = req.params;
    const portfolio = await getPortfolioData(accountId);

    res.json({
      success: true,
      portfolio: {
        id: portfolio.id,
        totalValue: portfolio.totalValue,
        currentAllocation: portfolio.currentAllocation,
        targetAllocation: portfolio.targetAllocation,
        lastRebalanced: portfolio.lastRebalanced,
        riskTolerance: portfolio.riskTolerance,
        holdingsCount: portfolio.holdings.length,
      },
    });
  } catch (error) {
    res.status(500).json({
      success: false,
      error: 'Failed to fetch portfolio status',
    });
  }
});

// Route to get market analysis
app.get('/api/market/analysis', async (req, res) => {
  try {
    const marketAnalysis = await analyzeMarketConditions();

    res.json({
      success: true,
      analysis: marketAnalysis,
      timestamp: new Date().toISOString(),
    });
  } catch (error) {
    res.status(500).json({
      success: false,
      error: 'Failed to fetch market analysis',
    });
  }
});

app.listen(3000, async () => {
  console.log('\n💹 Investment Portfolio Rebalancing System Starting...');
  console.log('⚡ Server running on port 3000');
  console.log('📍 Available endpoints:');
  console.log(
    '  POST /api/portfolio/rebalance - Trigger portfolio rebalancing'
  );
  console.log('  GET  /api/portfolio/:accountId/status - Get portfolio status');
  console.log('  GET  /api/market/analysis - Get current market analysis');

  await cronflow.start();
  console.log('🚀 Portfolio rebalancing workflows ready!');
});
```
