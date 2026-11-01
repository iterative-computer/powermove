(()=>{
  function square(x){return x*x+1}
  %PrepareFunctionForOptimization(square);
  for(let i=0;i<10000;i++)square(i);
  %OptimizeFunctionOnNextCall(square);
  if(square(6)!==37)throw Error('JIT value');
  const status=%GetOptimizationStatus(square);
  if(!(status&16))throw Error('optimized-code status absent: '+status);
  return JSON.stringify({probe:'signed-jit',optimized:true,status});
})()
