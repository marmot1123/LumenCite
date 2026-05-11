// Sample bibliographic data for LumenCite
// Real public paper metadata used for realism

const ENTRIES = [
  {
    id: 1, type: "article", title: "Attention Is All You Need",
    authors: ["Vaswani, A.", "Shazeer, N.", "Parmar, N.", "Uszkoreit, J.", "Jones, L.", "Gomez, A. N.", "Kaiser, Ł.", "Polosukhin, I."],
    year: 2017, venue: "NeurIPS", arxiv: "1706.03762", doi: "10.48550/arXiv.1706.03762",
    tags: ["transformer", "attention", "seminal"], collections: ["Transformer 系", "サーベイ"],
    added: "2025-09-12", read: true, attached: true, starred: true,
    abstract: "The dominant sequence transduction models are based on complex recurrent or convolutional neural networks. We propose a new simple network architecture, the Transformer, based solely on attention mechanisms, dispensing with recurrence and convolutions entirely.",
    notes: "TransformerはBERT・GPT系列の基礎。MultiHead Attentionの定式化を確認。"
  },
  {
    id: 2, type: "article", title: "BERT: Pre-training of Deep Bidirectional Transformers for Language Understanding",
    authors: ["Devlin, J.", "Chang, M.-W.", "Lee, K.", "Toutanova, K."],
    year: 2019, venue: "NAACL-HLT", arxiv: "1810.04805",
    tags: ["nlp", "pretraining", "transformer"], collections: ["Transformer 系"],
    added: "2025-09-14", read: true, attached: true, starred: false,
    abstract: "We introduce a new language representation model called BERT, which stands for Bidirectional Encoder Representations from Transformers."
  },
  {
    id: 3, type: "article", title: "Deep Residual Learning for Image Recognition",
    authors: ["He, K.", "Zhang, X.", "Ren, S.", "Sun, J."],
    year: 2016, venue: "CVPR", arxiv: "1512.03385",
    tags: ["cv", "resnet"], collections: ["サーベイ"],
    added: "2025-09-20", read: true, attached: true, starred: true,
  },
  {
    id: 4, type: "article", title: "Adam: A Method for Stochastic Optimization",
    authors: ["Kingma, D. P.", "Ba, J."],
    year: 2015, venue: "ICLR", arxiv: "1412.6980",
    tags: ["optimizer", "training"], collections: [],
    added: "2025-10-02", read: true, attached: true, starred: false,
  },
  {
    id: 5, type: "article", title: "Generative Adversarial Networks",
    authors: ["Goodfellow, I.", "Pouget-Abadie, J.", "Mirza, M.", "Xu, B.", "Warde-Farley, D.", "Ozair, S.", "Courville, A.", "Bengio, Y."],
    year: 2014, venue: "NeurIPS", arxiv: "1406.2661",
    tags: ["gan", "generative"], collections: [],
    added: "2025-10-05", read: false, attached: true, starred: false,
  },
  {
    id: 6, type: "article", title: "Language Models are Few-Shot Learners",
    authors: ["Brown, T. B.", "Mann, B.", "Ryder, N.", "Subbiah, M.", "Kaplan, J.", "Dhariwal, P.", "Neelakantan, A.", "et al."],
    year: 2020, venue: "NeurIPS", arxiv: "2005.14165",
    tags: ["llm", "gpt-3", "in-context"], collections: ["輪読会", "引用候補"],
    added: "2025-10-11", read: true, attached: true, starred: true,
    abstract: "We show that scaling up language models greatly improves task-agnostic, few-shot performance, sometimes even reaching competitiveness with prior state-of-the-art fine-tuning approaches."
  },
  {
    id: 7, type: "article", title: "Denoising Diffusion Probabilistic Models",
    authors: ["Ho, J.", "Jain, A.", "Abbeel, P."],
    year: 2020, venue: "NeurIPS", arxiv: "2006.11239",
    tags: ["diffusion", "generative"], collections: ["Diffusion 系", "輪読会"],
    added: "2025-10-14", read: true, attached: true, starred: true,
  },
  {
    id: 8, type: "article", title: "Auto-Encoding Variational Bayes",
    authors: ["Kingma, D. P.", "Welling, M."],
    year: 2014, venue: "ICLR", arxiv: "1312.6114",
    tags: ["vae", "generative"], collections: [],
    added: "2025-10-18", read: true, attached: true, starred: false,
  },
  {
    id: 9, type: "article", title: "ImageNet Classification with Deep Convolutional Neural Networks",
    authors: ["Krizhevsky, A.", "Sutskever, I.", "Hinton, G. E."],
    year: 2012, venue: "NeurIPS",
    tags: ["cv", "alexnet", "seminal"], collections: ["サーベイ"],
    added: "2025-10-22", read: true, attached: true, starred: false,
  },
  {
    id: 10, type: "article", title: "Sequence to Sequence Learning with Neural Networks",
    authors: ["Sutskever, I.", "Vinyals, O.", "Le, Q. V."],
    year: 2014, venue: "NeurIPS", arxiv: "1409.3215",
    tags: ["nlp", "seq2seq"], collections: [],
    added: "2025-10-25", read: true, attached: false, starred: false,
  },
  {
    id: 11, type: "article", title: "Long Short-Term Memory",
    authors: ["Hochreiter, S.", "Schmidhuber, J."],
    year: 1997, venue: "Neural Computation", doi: "10.1162/neco.1997.9.8.1735",
    tags: ["rnn", "classic"], collections: [],
    added: "2025-11-01", read: false, attached: false, starred: false,
  },
  {
    id: 12, type: "article", title: "Distributed Representations of Words and Phrases and their Compositionality",
    authors: ["Mikolov, T.", "Sutskever, I.", "Chen, K.", "Corrado, G.", "Dean, J."],
    year: 2013, venue: "NeurIPS", arxiv: "1310.4546",
    tags: ["nlp", "embeddings"], collections: [],
    added: "2025-11-04", read: true, attached: true, starred: false,
  },
  {
    id: 13, type: "article", title: "Mastering the game of Go with deep neural networks and tree search",
    authors: ["Silver, D.", "Huang, A.", "Maddison, C. J.", "Guez, A.", "Sifre, L.", "et al."],
    year: 2016, venue: "Nature", doi: "10.1038/nature16961",
    tags: ["rl", "alphago"], collections: [],
    added: "2025-11-09", read: true, attached: true, starred: true,
  },
  {
    id: 14, type: "book", title: "Pattern Recognition and Machine Learning",
    authors: ["Bishop, C. M."],
    year: 2006, venue: "Springer", isbn: "978-0387310732",
    tags: ["textbook", "ml"], collections: [],
    added: "2025-11-12", read: false, attached: true, starred: false,
  },
  {
    id: 15, type: "book", title: "Reinforcement Learning: An Introduction",
    authors: ["Sutton, R. S.", "Barto, A. G."],
    year: 2018, venue: "MIT Press (2nd ed.)", isbn: "978-0262039246",
    tags: ["rl", "textbook"], collections: [],
    added: "2025-11-15", read: false, attached: true, starred: true,
  },
  {
    id: 16, type: "webpage", title: "The Bitter Lesson",
    authors: ["Sutton, R. S."],
    year: 2019, venue: "incompleteideas.net",
    url: "http://www.incompleteideas.net/IncIdeas/BitterLesson.html",
    tags: ["ai", "essay", "philosophy"], collections: ["引用候補"],
    added: "2025-11-19", read: true, attached: false, starred: true,
  },
  {
    id: 17, type: "article", title: "Layer Normalization",
    authors: ["Ba, J. L.", "Kiros, J. R.", "Hinton, G. E."],
    year: 2016, arxiv: "1607.06450",
    tags: ["normalization"], collections: [],
    added: "2025-11-23", read: false, attached: true, starred: false,
  },
  {
    id: 18, type: "article", title: "Dropout: A Simple Way to Prevent Neural Networks from Overfitting",
    authors: ["Srivastava, N.", "Hinton, G.", "Krizhevsky, A.", "Sutskever, I.", "Salakhutdinov, R."],
    year: 2014, venue: "JMLR",
    tags: ["regularization"], collections: [],
    added: "2025-11-28", read: true, attached: true, starred: false,
  },
  {
    id: 19, type: "article", title: "Visualizing Data using t-SNE",
    authors: ["van der Maaten, L.", "Hinton, G."],
    year: 2008, venue: "JMLR",
    tags: ["visualization"], collections: [],
    added: "2025-12-03", read: false, attached: true, starred: false,
  },
  {
    id: 20, type: "inproceedings", title: "LoRA: Low-Rank Adaptation of Large Language Models",
    authors: ["Hu, E. J.", "Shen, Y.", "Wallis, P.", "Allen-Zhu, Z.", "Li, Y.", "Wang, S.", "Wang, L.", "Chen, W."],
    year: 2022, venue: "ICLR", arxiv: "2106.09685",
    tags: ["llm", "finetuning", "peft"], collections: ["引用候補", "輪読会"],
    added: "2025-12-08", read: true, attached: true, starred: true,
    abstract: "We propose Low-Rank Adaptation, or LoRA, which freezes the pre-trained model weights and injects trainable rank decomposition matrices into each layer of the Transformer architecture, greatly reducing the number of trainable parameters."
  },
  {
    id: 21, type: "article", title: "An Image is Worth 16x16 Words: Transformers for Image Recognition at Scale",
    authors: ["Dosovitskiy, A.", "Beyer, L.", "Kolesnikov, A.", "Weissenborn, D.", "Zhai, X.", "et al."],
    year: 2021, venue: "ICLR", arxiv: "2010.11929",
    tags: ["cv", "transformer", "vit"], collections: ["Transformer 系"],
    added: "2025-12-14", read: true, attached: true, starred: false,
  },
  {
    id: 22, type: "thesis", title: "Convolutional Networks for Biomedical Image Segmentation",
    authors: ["Ronneberger, O."],
    year: 2015, venue: "TU München (Habilitation)",
    tags: ["cv", "medical", "unet"], collections: [],
    added: "2026-01-08", read: false, attached: false, starred: false,
  },
  {
    id: 23, type: "article", title: "Constitutional AI: Harmlessness from AI Feedback",
    authors: ["Bai, Y.", "Kadavath, S.", "Kundu, S.", "Askell, A.", "et al."],
    year: 2022, arxiv: "2212.08073",
    tags: ["llm", "alignment", "rlhf"], collections: ["引用候補"],
    added: "2026-01-22", read: true, attached: true, starred: false,
  },
  {
    id: 24, type: "article", title: "Chain-of-Thought Prompting Elicits Reasoning in Large Language Models",
    authors: ["Wei, J.", "Wang, X.", "Schuurmans, D.", "Bosma, M.", "Ichter, B.", "et al."],
    year: 2022, venue: "NeurIPS", arxiv: "2201.11903",
    tags: ["llm", "reasoning", "prompting"], collections: ["輪読会"],
    added: "2026-02-04", read: true, attached: true, starred: true,
  },
  {
    id: 25, type: "misc", title: "PyTorch 2.0: An Open Source Machine Learning Framework",
    authors: ["Paszke, A.", "et al."],
    year: 2023, url: "https://pytorch.org",
    tags: ["framework", "pytorch"], collections: [],
    added: "2026-02-19", read: false, attached: false, starred: false,
  },
];

// Collections tree
const COLLECTIONS = [
  { id: "c1", name: "修士論文", icon: "folder", children: [
    { id: "c1a", name: "Transformer 系", icon: "folder", count: 4 },
    { id: "c1b", name: "Diffusion 系", icon: "folder", count: 2 },
  ]},
  { id: "c2", name: "輪読会", icon: "folder", count: 4 },
  { id: "c3", name: "引用候補", icon: "folder", count: 4 },
  { id: "c4", name: "サーベイ", icon: "folder", count: 3 },
];

const TAGS_USED = [
  { name: "transformer", color: "amber", count: 4 },
  { name: "nlp", color: "blue", count: 5 },
  { name: "cv", color: "green", count: 4 },
  { name: "generative", color: "violet", count: 3 },
  { name: "llm", color: "rose", count: 4 },
  { name: "rl", color: "cyan", count: 2 },
  { name: "seminal", color: "amber", count: 2 },
];

window.LUMEN_DATA = { ENTRIES, COLLECTIONS, TAGS_USED };
