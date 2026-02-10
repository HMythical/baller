# Contributions 

Baller is a brand new CLI package-manager that will potentially be able to be installed in Linux **AND** Windows.
We've just barely started, it's completely bare bones right now, and has a TON of potential to be a great piece
of sofware. Similarly, there is **alot** that can go wrong with this kind of software, especially given the goal of dual-os capabilities.
The main inspiration for this project is [Chocolatey](https://github.com/chocolatey/choco/tree/develop),
so this project will follow their guidelines on **pull requests**, **code contributions**, **documentation**, and most importantly....**TESTING**

Because this project is very new, potentially dangerous, and has insane potential to one of the greatest package managers for windows, besides chocolatey.
I will explicitly be a pain in the butt when it comes to exactly how much testing you do AND how you write committ messages (I'm not kidding).
A great example of great docuentation on a Pull Request can be found [here](https://github.com/chocolatey/choco/pull/3011).

In addition to that, I ask that you upload pictures/videos/visuals/wtv of your changes running with **NO UNINTENDED EXECUTION BEHAVIOR**... **WHATSOEVER**
Whatever contributuons that you decide to add to baller, you need to 110% sure that it's secure,safe, and easy to understand for other contributors.

Ultimately, all I ask is that you document everything. Code, Tests, Runtime, etc. Anything you want to add, write understandable documentation for it.



## What are you here for?

Baller (or Binary Allocation & Library Launch Environment in Rust if you want to be a nerd about it...), is going to have many components in its codebase,
two seperate operating-system environments, and so much more. You need to know exactly what you are contributing to, and exactly how you're code works when you PR.

### Submitting an Enhancement / Feature Request or Optimizing a function within Baller

If you're looking to improve Baller, then you're in the right place! Welcome! Make sure to familiarize yourself with Chocolatey's practices on PRs and Contributions as baller will implement
several of their practices during development of Baller. Please read the note at the bottom of the section.


### Linux or Windows? Which one?

Baller is a dual-os package-manager. That by-itself is a complex thing to think about. I ask that if you DO contribute to this project. Choose one envrionment and stick t it.
You might have experience in the Linux Kernel, or you might be a certified Windows pro, or you might just be the king of low-level software. That doesnt matter in this project.
The goal is to create a beautiful piece of software. I do not care about how long the development cycle takes. And you shouldn't either. High quality and secure code is the expectation,
and I'd rather you stick to one environment and PR solid code, than write below-decent code but get alot done. 

Essentialy, choose one. If you want to develop Baller in linux, then your contributions, even the ones aftr your initial contribution, should stay in Linux. Same for windows. Security > Speed.

If your PR has changes for both environments, your contribution may not be accepted. So please stick to one operating system environment! Trust me, you will hate it, but I'm adding this rule for a reason.

### **PLEASE NOTE: YOU NEED TO LOG A GITHUB ISSUE IF YOU ARE SUBMITTING AN ENHANCEMENT**...
Its because there are less constraints, rather than reporting an issue. 



### Pre-requisites

- Fork the repo
- Sign the Contributor License Agreement (CLA) - I'm not trying to get in legal trouble. I checked the CLA, it should not have predatory legal language in any way, shape, or form.
I will not accept any contributions without it.
- Sign it for each Baller project that required it. Simple.
- Why am I having you sign this? Julien Ponge. Specifically section 5.1 of his blog post. Reference his post [here](https://julien.ponge.org/blog/in-defense-of-contributor-license-agreements/).
- [Sign the CLA Here](https://cla-assistant.io/HMythical/baller)

#### Do I really have to sign the CLA?
Yes. End of discussion.




# Sooo...How do I... you know...contribute? 
## Choose an environment

As previously stated before, If you want to contribute to baller, you will need to contribute to ONE operating-system environment. Windows OR Linux, one or the other. The only exception to this rule will be for the OS-detection service. That will be just rust with libraries and stuff. 



### Rust
Rust will be the **main** language used for this project. Rust was chosen because of it's memory saftey compared to C and C++ as well as its compatibility with both Linux and Windows (I chose rust primarily because of this reason). However, rust isnt just the only language we will have to use to make this package manager work!

### Powershell
Windows is weird and doesnt use bash normally. We all know about cmd and PowerShell... All contributitions that relate to powershell, rust or something else, must be able to work with Powershell v3 (v2 we can get to eventually). 

### C?? C#?? .NET?? What about those?
Honestly, I am not opposed to using those languages, however, I do not know exactly how C and Rust would interact if we used them bot. Same thing with C# and the .NET Framework. I am not opposed to contributions with those languages, but you the contributor as well as I need to know how they will interact under the hood. If you do use these languages, make sure to document,compare, and summarize how different the assembly instructions are and make sure the instructions that come out of compilation do not cause memory errors.

Additionally... **please write memory safe code**... I'm going to leave it at that....


### Should I use other languages like Go or Ruby or something else?

No. I only allow languages I understand under the hood, or I can easily learn how they work under the hood.

## Code Formatting / Design

Until contributions start to pile in in **different** languages, the rust files will have configuration files to enforce certain formatting standards.

SOME naming conventions will be listed here for youre reference. It will also be in the Baller Docs (when I get around to creating them) as well as an '.editorconfig' file when I make that:

- Non-OS specific Structs must all be PascalCase, with context included. [Context][Purpose][TypeSuffix]
- All variables must have explicit declaration
- Unless you require OS-specific behavior, use Rust primitives DIRECTLY
- Non-OS specific variables must be in snake_case and explicitly declared
- Operating System specific variables must be prefixed with "os_"
- Collections and Tuples must be plural
- Booleans must be prefixed with is_,has_,can_,should_ and their grammatical opposites when dealing with false Boolean values (isnt_,hasnt_,cant_,etc)
- OS-specific Structs must be prefixed with their respective OS. (E.g LinuxPackageInfo)
- Configuration structs must be suffixed with Config (E.g InstallConfig)
- Builders must be suffixed with Builder (E.g PackageQueryBuilder)
- Data Transfer Objects must be suffixed with DTO
- Errors must be suffixed with their OS
  e.g

  pub enum InstallError{
    Linux(LinuxError),
    Windows(WindowsError),
    Common(CommonError),

  }

  The rest will be present in the sources previously mentioned

### Setting your Environment Up

- Git Specific information
    1. Create a fork of HMythical/baller under your Github Account
    2. Clone YOUR fork locally
    3. Open a command line and navigate to that directory
    4. Add the upstream fork - ' git remote add upstream git@github.com:HMythical/baller.git'
    5. Run ' git fetch upstream
    6. Ensure you have your user name and your email set up correctly to atribute your contributions
    7. Create a branch named specifically to what you are contributing
    8. Do your work for the specifc branch
    9. When you are done and want to commit, reference the [**Commit Survival Guide**](#The-Survival-Guide-for-Commits-(I'm-"stealing"-chocolatey's-framework-for-commits...-please-dont-sue-me) )

## Documenting,Debugging and Testing




### How should I document my code?
Please make sure you document the code you write. Leave meaningful and useful comments, inline comments can be used to explain certain variables incase other contributors decide to join in.

Also document how you tested your changes, what the outputs were, what you expected, and the constraints you had. Documented tests are one of the most important that need to be inside your commit message.

Essentially, write down everything you change.

### How should I test it?

Honestly, since the project has just started, there is no concrete way to test what you created. For now, whatever changes you make, put them in a separate project and run them using your IDE's debug features. I would also make sure you know exactly what your change is supposed to do so you can write the tests separate. Make sure you test for normal cases and edge cases as well.

## So... about my IDE... which one should I use?!

I have no preference for what IDE you should use. Chocolatey uses Visual Studio 2019+, but I have no clue why they chose that one specifically. I would reccomend a decent JetBrains IDE if you are working on the Linux environment, if not, you can just use NeoVim or some other IDE. If you're working in the Windows environment... honestly have no clue. That'll be hashed out eventually.

## The Survival Guide for Commits (Directly from Chocolatey's guide on commit messages.)

.... Go read [this](https://github.com/chocolatey/choco/edit/develop/CONTRIBUTING.md#prepare-commits)

Chocolatey has an insanely good guide on commits. So use theirs. Thats it. Only difference is that commit messages for baller should (and will be) way longer. It should be a detailed message.

Example of a commit message that I'd be okay with:

```
(#7) Refactor Libraries in /linux/src/lib.rs and Entry-Point Logic

Previous versions of the entry-point logic in main.rs work for certain
windows and linux kernels. It does not work on Debian 11 and Windows
10 due to the absenceof key OS system calls and other nessecary APIs
within both operating systems. Additionally, certain libraries inside
of the linux environment will update to accomodate for older versions
of the two operating systems.


Documentation & Tests:

[Insert Documentation here]

[Insert More Documentation here]

[Insert tests here]

If this change does not go through, key systems such as the
DependencyManager will fail, leading to unsafe execution and possible
memory corruption.

```


### The Pull Request

Generally, just follow what Chocolatey does [here](https://github.com/chocolatey/choco/edit/develop/CONTRIBUTING.md#submit-pull-request-pr)

### Feedback?

If your commit message and Pull Request are genuinely unreadable and I cant understand it, then I'll send it back and have you explain more clearly. The key is that you explain what you added so I/Other contributors (You see what I did there?) can add onto what you contributed and build more efficiently.

## I like how Chocolatey does "x" thing... Can we "implement" it like they do?
No. Dont copy-paste code. If you contributed to Chocolatey and now you're contributing here... Hi, Im a big fan, would love to have your contributions... But if you just copy paste their code here. Absolutely not. 

Additionally, if you copy-paste code from any other repo without their permission, I am both legally, and morally obligated to report you to them directly. So please dont.




# Other important general information

This contributions file is definetly going to change as Baller gains contributrs and development time. This file is meant to lay down the framework for how Baller will want to grow in order to be a reliable piece of software. 



# Conclusion

**Your contributions will never be forgotten! Thank you for putting your time, energy, and you passion into this project. I am eternally grateful for any who decide to help make Baller come to life! You're work will pave the way for this software to grow exponentially!**


